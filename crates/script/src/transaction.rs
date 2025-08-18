use super::ScriptResult;
use crate::build::LinkedBuildData;
use alloy_dyn_abi::JsonAbiExt;
use alloy_primitives::{Address, B256, TxKind, hex};
use eyre::Result;
use forge_script_sequence::TransactionWithMetadata;
use foundry_common::{ContractData, SELECTOR_LEN, TransactionMaybeSigned, fmt::format_token_raw};
use foundry_evm::traces::CallTraceDecoder;
use itertools::Itertools;
use revm_inspectors::tracing::types::CallKind;
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct ScriptTransactionBuilder {
    transaction: TransactionWithMetadata,
}

impl ScriptTransactionBuilder {
    pub fn new(transaction: TransactionMaybeSigned, rpc: String) -> Self {
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
        if let Some(TxKind::Call(to)) = self.transaction.transaction.to() {
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
                self.transaction.opcode = CallKind::Call;
                self.transaction.contract_address = Some(to);

                let Some(data) = self.transaction.transaction.input() else { return Ok(()) };

                if data.len() < SELECTOR_LEN {
                    return Ok(());
                }

                let (selector, data) = data.split_at(SELECTOR_LEN);

                let function = if let Some(info) = local_contracts.get(&to) {
                    // This CALL is made to a local contract.
                    self.transaction.contract_name = Some(info.name.clone());
                    info.abi.functions().find(|function| function.selector() == selector)
                } else {
                    // This CALL is made to an external contract; try to decode it from the given
                    // decoder.
                    decoder.functions.get(selector).and_then(|v| v.first())
                };

                if let Some(function) = function {
                    self.transaction.function = Some(function.signature());

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
            self.transaction.opcode = CallKind::Create2;
        } else {
            self.transaction.opcode = CallKind::Create;
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
        result: &ScriptResult,
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
            unsigned.gas = Some(result.gas_used * gas_estimate_multiplier / 100);
        }

        self
    }

    pub fn build(self) -> TransactionWithMetadata {
        self.transaction
    }
}

impl From<TransactionWithMetadata> for ScriptTransactionBuilder {
    fn from(transaction: TransactionWithMetadata) -> Self {
        Self { transaction }
    }
}
