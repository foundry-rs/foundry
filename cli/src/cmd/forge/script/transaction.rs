use crate::cmd::forge::script::ScriptResult;
use cast::{executor::inspector::DEFAULT_CREATE2_DEPLOYER, trace::CallTraceDecoder, CallKind};
use ethers::{
    abi,
    abi::{Address, Contract as Abi},
    prelude::{NameOrAddress, H256 as TxHash},
    types::transaction::eip2718::TypedTransaction,
};
use foundry_common::SELECTOR_LEN;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Deserialize, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct AdditionalContract {
    #[serde(rename = "transactionType")]
    pub opcode: CallKind,
    pub address: Address,
    #[serde(with = "hex")]
    pub init_code: Vec<u8>,
}

#[derive(Deserialize, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct TransactionWithMetadata {
    pub hash: Option<TxHash>,
    #[serde(rename = "transactionType")]
    pub opcode: CallKind,
    #[serde(default = "default_string")]
    pub contract_name: Option<String>,
    #[serde(default = "default_address")]
    pub contract_address: Option<Address>,
    #[serde(default = "default_string")]
    pub function: Option<String>,
    #[serde(default = "default_vec_of_strings")]
    pub arguments: Option<Vec<String>>,
    pub transaction: TypedTransaction,
    pub additional_contracts: Vec<AdditionalContract>,
}

fn default_string() -> Option<String> {
    Some("".to_string())
}

fn default_address() -> Option<Address> {
    Some(Address::zero())
}

fn default_vec_of_strings() -> Option<Vec<String>> {
    Some(vec![])
}

impl TransactionWithMetadata {
    pub fn from_typed_transaction(transaction: TypedTransaction) -> Self {
        Self { transaction, ..Default::default() }
    }

    pub fn new(
        transaction: TypedTransaction,
        result: &ScriptResult,
        local_contracts: &BTreeMap<Address, (String, &Abi)>,
        decoder: &CallTraceDecoder,
        additional_contracts: Vec<AdditionalContract>,
    ) -> eyre::Result<Self> {
        let mut metadata = Self { transaction, ..Default::default() };

        // Specify if any contract was directly created with this transaction
        if let Some(NameOrAddress::Address(to)) = metadata.transaction.to().cloned() {
            if to == DEFAULT_CREATE2_DEPLOYER {
                metadata.set_create(
                    true,
                    Address::from_slice(&result.returned),
                    local_contracts,
                    decoder,
                )?;
            } else {
                metadata.set_call(to, local_contracts, decoder)?;
            }
        } else if metadata.transaction.to().is_none() {
            metadata.set_create(
                false,
                result.address.expect("There should be a contract address."),
                local_contracts,
                decoder,
            )?;
        }

        // Add the additional contracts created in this transaction, so we can verify them later.
        if let Some(tx_address) = metadata.contract_address {
            metadata.additional_contracts = additional_contracts
                .into_iter()
                .filter_map(|contract| {
                    // Filter out the transaction contract repeated init_code.
                    if contract.address != tx_address {
                        Some(contract)
                    } else {
                        None
                    }
                })
                .collect();
        }

        Ok(metadata)
    }

    /// Populate the transaction as CREATE tx
    ///
    /// If this is a CREATE2 transaction this attempt to decode the arguments from the CREATE2
    /// deployer's function
    fn set_create(
        &mut self,
        is_create2: bool,
        address: Address,
        contracts: &BTreeMap<Address, (String, &Abi)>,
        decoder: &CallTraceDecoder,
    ) -> eyre::Result<()> {
        if is_create2 {
            self.opcode = CallKind::Create2;
        } else {
            self.opcode = CallKind::Create;
        }

        self.contract_name = contracts.get(&address).map(|(name, _)| name.clone());
        self.contract_address = Some(address);

        if let Some(data) = self.transaction.data() {
            // a CREATE2 transaction is a CALL to the CREATE2 deployer function, so we need to
            // decode the arguments  from the function call
            if is_create2 {
                // this will be the `deploy()` function of the CREATE2 call, we assume the contract
                // is already deployed and will try to fetch it via its signature
                if let Some(Some(function)) = decoder
                    .functions
                    .get(&data.0[..SELECTOR_LEN])
                    .map(|functions| functions.first())
                {
                    self.function = Some(function.signature());
                    self.arguments =
                        Some(function.decode_input(&data.0[SELECTOR_LEN..]).map(|tokens| {
                            tokens.iter().map(|token| token.to_string()).collect()
                        })?);
                }
            } else {
                // This is a regular CREATE via the constructor, in which case we expect the
                // contract has a constructor and if so decode its input params
                if let Some((_, abi)) = contracts.get(&address) {
                    // set constructor arguments
                    if let Some(constructor) = abi.constructor() {
                        let params =
                            constructor.inputs.iter().map(|p| p.kind.clone()).collect::<Vec<_>>();
                        self.arguments = Some(
                            abi::decode(&params, &data.0)?
                                .into_iter()
                                .map(|token| token.to_string())
                                .collect(),
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Populate the transaction as CALL tx
    fn set_call(
        &mut self,
        target: Address,
        local_contracts: &BTreeMap<Address, (String, &Abi)>,
        decoder: &CallTraceDecoder,
    ) -> eyre::Result<()> {
        self.opcode = CallKind::Call;

        if let Some(data) = self.transaction.data() {
            if data.0.len() >= SELECTOR_LEN {
                if let Some((contract_name, abi)) = local_contracts.get(&target) {
                    // This CALL is made to a local contract.

                    self.contract_name = Some(contract_name.clone());
                    if let Some(function) = abi
                        .functions()
                        .find(|function| function.short_signature() == data.0[..SELECTOR_LEN])
                    {
                        self.function = Some(function.signature());
                        self.arguments =
                            Some(function.decode_input(&data.0[SELECTOR_LEN..]).map(|tokens| {
                                tokens.iter().map(|token| token.to_string()).collect()
                            })?);
                    }
                } else {
                    // This CALL is made to an external contract. We can only decode it, if it has
                    // been verified and identified by etherscan.

                    if let Some(Some(function)) = decoder
                        .functions
                        .get(&data.0[..SELECTOR_LEN])
                        .map(|functions| functions.first())
                    {
                        self.contract_name = decoder.contracts.get(&target).cloned();

                        self.function = Some(function.signature());
                        self.arguments =
                            Some(function.decode_input(&data.0[SELECTOR_LEN..]).map(|tokens| {
                                tokens.iter().map(|token| token.to_string()).collect()
                            })?);
                    }
                }
                self.contract_address = Some(target);
            }
        }
        Ok(())
    }

    pub fn set_tx(&mut self, tx: TypedTransaction) {
        self.transaction = tx;
    }

    pub fn change_type(&mut self, is_legacy: bool) {
        self.transaction = if is_legacy {
            TypedTransaction::Legacy(self.transaction.clone().into())
        } else {
            TypedTransaction::Eip1559(self.transaction.clone().into())
        };
    }

    pub fn typed_tx(&self) -> &TypedTransaction {
        &self.transaction
    }

    pub fn typed_tx_mut(&mut self) -> &mut TypedTransaction {
        &mut self.transaction
    }

    pub fn is_create2(&self) -> bool {
        self.opcode == CallKind::Create2
    }
}
