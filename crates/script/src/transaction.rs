use super::ScriptResult;
use alloy_dyn_abi::JsonAbiExt;
use alloy_primitives::{hex, Address, Bytes, TxKind, B256};
use eyre::{Result, WrapErr};
use foundry_common::{fmt::format_token_raw, ContractData, TransactionMaybeSigned, SELECTOR_LEN};
use foundry_evm::{constants::DEFAULT_CREATE2_DEPLOYER, traces::CallTraceDecoder};
use itertools::Itertools;
use revm_inspectors::tracing::types::CallKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdditionalContract {
    #[serde(rename = "transactionType")]
    pub opcode: CallKind,
    pub address: Address,
    pub init_code: Bytes,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionWithMetadata {
    pub hash: Option<B256>,
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
    #[serde(skip)]
    pub rpc: String,
    pub transaction: TransactionMaybeSigned,
    pub additional_contracts: Vec<AdditionalContract>,
    pub is_fixed_gas_limit: bool,
}

fn default_string() -> Option<String> {
    Some(String::new())
}

fn default_address() -> Option<Address> {
    Some(Address::ZERO)
}

fn default_vec_of_strings() -> Option<Vec<String>> {
    Some(vec![])
}

impl TransactionWithMetadata {
    pub fn from_tx_request(transaction: TransactionMaybeSigned) -> Self {
        Self {
            transaction,
            hash: Default::default(),
            opcode: Default::default(),
            contract_name: Default::default(),
            contract_address: Default::default(),
            function: Default::default(),
            arguments: Default::default(),
            is_fixed_gas_limit: Default::default(),
            additional_contracts: Default::default(),
            rpc: Default::default(),
        }
    }

    pub fn new(
        transaction: TransactionMaybeSigned,
        rpc: String,
        local_contracts: &BTreeMap<Address, &ContractData>,
        decoder: &CallTraceDecoder,
    ) -> Result<Self> {
        let mut metadata = Self::from_tx_request(transaction);
        metadata.rpc = rpc;
        // If tx.gas is already set that means it was specified in script
        metadata.is_fixed_gas_limit = metadata.tx().gas().is_some();

        if let Some(TxKind::Call(to)) = metadata.transaction.to() {
            if to == DEFAULT_CREATE2_DEPLOYER {
                if let Some(input) = metadata.transaction.input() {
                    let (salt, init_code) = input.split_at(32);
                    metadata.set_create(
                        true,
                        DEFAULT_CREATE2_DEPLOYER
                            .create2_from_code(B256::from_slice(salt), init_code),
                        local_contracts,
                    )?;
                }
            } else {
                metadata
                    .set_call(to, local_contracts, decoder)
                    .wrap_err("Could not decode transaction type.")?;
            }
        } else {
            let sender =
                metadata.transaction.from().expect("all transactions should have a sender");
            let nonce = metadata.transaction.nonce().expect("all transactions should have a nonce");
            metadata.set_create(false, sender.create(nonce), local_contracts)?;
        }

        Ok(metadata)
    }

    /// Populates additional data from the transaction execution result.
    pub fn with_execution_result(
        mut self,
        result: &ScriptResult,
        gas_estimate_multiplier: u64,
    ) -> Self {
        let mut created_contracts = result.get_created_contracts();

        // Add the additional contracts created in this transaction, so we can verify them later.
        created_contracts.retain(|contract| {
            // Filter out the contract that was created by the transaction itself.
            self.contract_address.map_or(true, |addr| addr != contract.address)
        });

        if !self.is_fixed_gas_limit {
            if let Some(unsigned) = self.transaction.as_unsigned_mut() {
                // We inflate the gas used by the user specified percentage
                unsigned.gas = Some((result.gas_used * gas_estimate_multiplier / 100) as u128);
            }
        }

        self
    }

    /// Populate the transaction as CREATE tx
    ///
    /// If this is a CREATE2 transaction this attempt to decode the arguments from the CREATE2
    /// deployer's function
    fn set_create(
        &mut self,
        is_create2: bool,
        address: Address,
        contracts: &BTreeMap<Address, &ContractData>,
    ) -> Result<()> {
        if is_create2 {
            self.opcode = CallKind::Create2;
        } else {
            self.opcode = CallKind::Create;
        }

        let info = contracts.get(&address);
        self.contract_name = info.map(|info| info.name.clone());
        self.contract_address = Some(address);

        let Some(data) = self.transaction.input() else { return Ok(()) };
        let Some(info) = info else { return Ok(()) };
        let Some(bytecode) = info.bytecode() else { return Ok(()) };

        // `create2` transactions are prefixed by a 32 byte salt.
        let creation_code = if is_create2 {
            if data.len() < 32 {
                return Ok(())
            }
            &data[32..]
        } else {
            data
        };

        // The constructor args start after bytecode.
        let contains_constructor_args = creation_code.len() > bytecode.len();
        if !contains_constructor_args {
            return Ok(());
        }
        let constructor_args = &creation_code[bytecode.len()..];

        let Some(constructor) = info.abi.constructor() else { return Ok(()) };
        let values = constructor.abi_decode_input(constructor_args, false).inspect_err(|_| {
            error!(
                contract=?self.contract_name,
                signature=%format!("constructor({})", constructor.inputs.iter().map(|p| &p.ty).format(",")),
                is_create2,
                constructor_args=%hex::encode(constructor_args),
                "Failed to decode constructor arguments",
            );
            debug!(full_data=%hex::encode(data), bytecode=%hex::encode(creation_code));
        })?;
        self.arguments = Some(values.iter().map(format_token_raw).collect());

        Ok(())
    }

    /// Populate the transaction as CALL tx
    fn set_call(
        &mut self,
        target: Address,
        local_contracts: &BTreeMap<Address, &ContractData>,
        decoder: &CallTraceDecoder,
    ) -> Result<()> {
        self.opcode = CallKind::Call;
        self.contract_address = Some(target);

        let Some(data) = self.transaction.input() else { return Ok(()) };
        if data.len() < SELECTOR_LEN {
            return Ok(());
        }
        let (selector, data) = data.split_at(SELECTOR_LEN);

        let function = if let Some(info) = local_contracts.get(&target) {
            // This CALL is made to a local contract.
            self.contract_name = Some(info.name.clone());
            info.abi.functions().find(|function| function.selector() == selector)
        } else {
            // This CALL is made to an external contract; try to decode it from the given decoder.
            decoder.functions.get(selector).and_then(|v| v.first())
        };
        if let Some(function) = function {
            self.function = Some(function.signature());

            let values = function.abi_decode_input(data, false).inspect_err(|_| {
                error!(
                    contract=?self.contract_name,
                    signature=?function,
                    data=hex::encode(data),
                    "Failed to decode function arguments",
                );
            })?;
            self.arguments = Some(values.iter().map(format_token_raw).collect());
        }

        Ok(())
    }

    pub fn tx(&self) -> &TransactionMaybeSigned {
        &self.transaction
    }

    pub fn tx_mut(&mut self) -> &mut TransactionMaybeSigned {
        &mut self.transaction
    }

    pub fn is_create2(&self) -> bool {
        self.opcode == CallKind::Create2
    }
}
