use super::ScriptResult;
use crate::build::LinkedBuildData;
use alloy_dyn_abi::JsonAbiExt;
use alloy_network::{Network, TransactionBuilder};
use alloy_primitives::{Address, B256, Selector, hex};
use eyre::Result;
use forge_script_sequence::TransactionWithMetadata;
use foundry_common::{ContractData, SELECTOR_LEN, TransactionMaybeSigned, fmt::format_token_raw};
use foundry_evm::traces::CallTraceDecoder;
use itertools::Itertools;
use revm_inspectors::tracing::types::CallKind;
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct ScriptTransactionBuilder<N: Network> {
    transaction: TransactionWithMetadata<N>,
}

impl<N: Network> ScriptTransactionBuilder<N> {
    pub fn new(transaction: TransactionMaybeSigned<N>, rpc: String) -> Self {
        let mut transaction = TransactionWithMetadata::from_tx_request(transaction);
        transaction.rpc = rpc;
        // If tx.gas is already set that means it was specified in script
        transaction.is_fixed_gas_limit = transaction.tx().gas().is_some();

        Self { transaction }
    }

    /// Populate the transaction as CALL tx
    pub fn set_call(
        &mut self,
        local_contracts: &BTreeMap<Address, &ContractData>,
        decoder: &CallTraceDecoder,
        create2_deployer: Address,
    ) -> Result<()> {
        if let Some(to) = self.transaction.transaction.to() {
            if to == create2_deployer {
                if let Some(input) = self.transaction.transaction.input() {
                    let (salt, init_code) = input.split_at(32);

                    self.set_create(
                        true,
                        create2_deployer.create2_from_code(B256::from_slice(salt), init_code),
                        local_contracts,
                    )?;
                }
            } else {
                self.transaction.call_kind = CallKind::Call;
                self.transaction.contract_address = Some(to);

                let Some(data) = self.transaction.transaction.input() else { return Ok(()) };

                if data.len() < SELECTOR_LEN {
                    return Ok(());
                }

                let (selector, data) = data.split_at(SELECTOR_LEN);
                let selector = Selector::from_slice(selector);

                let function = if let Some(info) = local_contracts.get(&to) {
                    // This CALL is made to a local contract.
                    self.transaction.contract_name = Some(info.name.clone());
                    info.abi.functions().find(|function| function.selector() == selector)
                } else {
                    // This CALL is made to an external contract; try to decode it from the given
                    // decoder.
                    decoder
                        .functions_for_selector(to, &selector)
                        .and_then(|functions| functions.first())
                };

                if let Some(function) = function {
                    self.transaction.function = Some(function.signature());
                    self.transaction.function_abi = Some(function.full_signature());
                    self.transaction.display_function = Some(function.name.clone());

                    let values = function.abi_decode_input(data).inspect_err(|_| {
                        error!(
                            contract=?self.transaction.contract_name,
                            signature=?function,
                            data=hex::encode(data),
                            "Failed to decode function arguments",
                        );
                    })?;
                    self.transaction.arguments =
                        Some(values.iter().map(format_token_raw).collect());
                }
            }
        }

        Ok(())
    }

    /// Populate the transaction as CREATE tx
    ///
    /// If this is a CREATE2 transaction this attempt to decode the arguments from the CREATE2
    /// deployer's function
    pub fn set_create(
        &mut self,
        is_create2: bool,
        address: Address,
        contracts: &BTreeMap<Address, &ContractData>,
    ) -> Result<()> {
        if is_create2 {
            self.transaction.call_kind = CallKind::Create2;
        } else {
            self.transaction.call_kind = CallKind::Create;
        }

        let info = contracts.get(&address);
        self.transaction.contract_name = info.map(|info| info.name.clone());
        self.transaction.contract_address = Some(address);

        let Some(data) = self.transaction.transaction.input() else { return Ok(()) };
        let Some(info) = info else { return Ok(()) };
        let Some(bytecode) = info.bytecode() else { return Ok(()) };

        // `create2` transactions are prefixed by a 32 byte salt.
        let creation_code = if is_create2 {
            if data.len() < 32 {
                return Ok(());
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
        let values = constructor.abi_decode_input(constructor_args).inspect_err(|_| {
                error!(
                    contract=?self.transaction.contract_name,
                    signature=%format!("constructor({})", constructor.inputs.iter().map(|p| &p.ty).format(",")),
                    is_create2,
                    constructor_args=%hex::encode(constructor_args),
                    "Failed to decode constructor arguments",
                );
                debug!(full_data=%hex::encode(data), bytecode=%hex::encode(creation_code));
            })?;
        self.transaction.arguments = Some(values.iter().map(format_token_raw).collect());

        Ok(())
    }

    /// Populates additional data from the transaction execution result.
    pub fn with_execution_result(
        mut self,
        result: &ScriptResult<N>,
        gas_estimate_multiplier: u64,
        linked_build_data: &LinkedBuildData,
    ) -> Self {
        let mut created_contracts =
            result.get_created_contracts(&linked_build_data.known_contracts);

        // Add the additional contracts created in this transaction, so we can verify them later.
        created_contracts.retain(|contract| {
            // Filter out the contract that was created by the transaction itself.
            self.transaction.contract_address != Some(contract.address)
        });

        self.transaction.additional_contracts = created_contracts;

        if !self.transaction.is_fixed_gas_limit
            && let Some(unsigned) = self.transaction.transaction.as_unsigned_mut()
        {
            // We inflate the gas used by the user specified percentage
            unsigned.set_gas_limit(result.gas_used * gas_estimate_multiplier / 100);
        }

        self
    }

    pub fn build(self) -> TransactionWithMetadata<N> {
        self.transaction
    }
}

impl<N: Network> From<TransactionWithMetadata<N>> for ScriptTransactionBuilder<N> {
    fn from(transaction: TransactionWithMetadata<N>) -> Self {
        Self { transaction }
    }
}

#[cfg(all(test, feature = "monad"))]
mod tests {
    use super::*;
    use alloy_network::Ethereum;
    use alloy_primitives::{Bytes, address, keccak256};
    use alloy_rpc_types::TransactionRequest;
    use foundry_evm::{hardforks::MonadHardfork, traces::CallTraceDecoderBuilder};
    use foundry_evm_networks::NetworkConfigs;

    const STAKING_ADDRESS: Address = address!("0000000000000000000000000000000000001000");
    const RESERVE_BALANCE_ADDRESS: Address = address!("0000000000000000000000000000000000001001");

    fn monad_decoder(hardfork: MonadHardfork) -> CallTraceDecoder {
        CallTraceDecoderBuilder::new()
            .with_networks(NetworkConfigs::with_monad())
            .with_chain_id(Some(143))
            .with_hardfork(Some(hardfork.into()))
            .build()
    }

    fn call_metadata(
        address: Address,
        signature: &str,
        hardfork: MonadHardfork,
    ) -> TransactionWithMetadata<Ethereum> {
        let input = Bytes::copy_from_slice(&keccak256(signature)[..SELECTOR_LEN]);
        let selector = Selector::from_slice(&input);
        let decoder = monad_decoder(hardfork);

        assert!(!decoder.functions.contains_key(&selector));
        assert!(decoder.functions_for_selector(address, &selector).is_some());

        let transaction = TransactionRequest::default()
            .with_from(Address::repeat_byte(0x11))
            .with_to(address)
            .with_nonce(0)
            .with_input(input);
        let mut builder = ScriptTransactionBuilder::new(
            TransactionMaybeSigned::new(transaction),
            "http://localhost:8545".to_string(),
        );
        builder.set_call(&BTreeMap::new(), &decoder, Address::ZERO).unwrap();
        builder.build()
    }

    #[test]
    fn address_scoped_monad_calls_populate_metadata() {
        let staking = call_metadata(STAKING_ADDRESS, "getEpoch()", MonadHardfork::MonadEight);
        assert_eq!(staking.function.as_deref(), Some("getEpoch()"));
        assert_eq!(
            staking.function_abi.as_deref(),
            Some("function getEpoch() returns (uint64 epoch, bool inEpochDelayPeriod)")
        );
        assert_eq!(staking.display_function.as_deref(), Some("getEpoch"));
        assert_eq!(staking.arguments, Some(Vec::new()));

        let reserve =
            call_metadata(RESERVE_BALANCE_ADDRESS, "dippedIntoReserve()", MonadHardfork::MonadNine);
        assert_eq!(reserve.function.as_deref(), Some("dippedIntoReserve()"));
        assert_eq!(
            reserve.function_abi.as_deref(),
            Some("function dippedIntoReserve() returns (bool dipped)")
        );
        assert_eq!(reserve.display_function.as_deref(), Some("dippedIntoReserve"));
        assert_eq!(reserve.arguments, Some(Vec::new()));
    }
}
