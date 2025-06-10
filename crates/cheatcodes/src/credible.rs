use crate::{Cheatcode, CheatcodesExecutor, CheatsCtxt, Result, Vm::*};
use alloy_primitives::TxKind;
use alloy_sol_types::{Revert, SolError, SolValue};
use assertion_executor::{
    db::fork_db::ForkDb,
    store::{AssertionState, AssertionStore},
    ExecutorConfig,
};
use foundry_evm_core::backend::{DatabaseError, DatabaseExt};
use revm::{
    primitives::{AccountInfo, Address, Bytecode, ExecutionResult, TxEnv, B256, U256},
    DatabaseCommit, DatabaseRef,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Wrapper around DatabaseExt to make it thread-safe
#[derive(Clone)]
struct ThreadSafeDb<'a> {
    db: Arc<Mutex<&'a mut dyn DatabaseExt>>,
}

impl std::fmt::Debug for ThreadSafeDb<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ThreadSafeDb")
    }
}

/// Separate implementation block for constructor and helper methods
impl<'a> ThreadSafeDb<'a> {
    /// Creates a new thread-safe database wrapper
    pub fn new(db: &'a mut dyn DatabaseExt) -> Self {
        Self { db: Arc::new(Mutex::new(db)) }
    }
}

/// Keep DatabaseRef implementation separate
impl<'a> DatabaseRef for ThreadSafeDb<'a> {
    type Error = DatabaseError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.db.lock().unwrap().basic(address)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.db.lock().unwrap().code_by_hash(code_hash)
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.db.lock().unwrap().storage(address, index)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.db.lock().unwrap().block_hash(number)
    }
}

impl Cheatcode for assertionExCall {
    fn apply_full(&self, ccx: &mut CheatsCtxt, executor: &mut dyn CheatcodesExecutor) -> Result {
        let Self {
            tx,
            assertionAdopter: assertion_adopter,
            assertionContract,
            assertionContractLabel,
        } = self;

        let spec_id = ccx.ecx.spec_id();
        let block = ccx.ecx.env.block.clone();
        let state = ccx.ecx.journaled_state.state.clone();
        let chain_id = ccx.ecx.env.cfg.chain_id;

        // Setup assertion database
        let db = ThreadSafeDb::new(ccx.ecx.db);

        // Prepare assertion store
        let assertion_contract_bytecode = Bytecode::LegacyRaw(assertionContract.to_vec().into());

        let config = ExecutorConfig { spec_id, chain_id, assertion_gas_limit: 100_000 };

        let store = AssertionStore::new_ephemeral().expect("Failed to create assertion store");

        let assertion_state =
            AssertionState::new_active(assertion_contract_bytecode.bytes(), &config)
                .expect("Failed to create assertion state");

        store.insert(*assertion_adopter, assertion_state).expect("Failed to store assertions");

        let decoded_tx = AssertionExTransaction::abi_decode(tx, true)?;

        let tx_env = TxEnv {
            caller: decoded_tx.from,
            gas_limit: ccx.ecx.env.block.gas_limit.try_into().unwrap_or(u64::MAX),
            transact_to: TxKind::Call(decoded_tx.to),
            value: decoded_tx.value,
            data: decoded_tx.data,
            chain_id: Some(chain_id),
            ..Default::default()
        };

        let mut assertion_executor = config.build(db, store);

        // Commit current journal state so that it is available for assertions and
        // triggering tx
        let mut fork_db = ForkDb::new(assertion_executor.db.clone());
        fork_db.commit(state);

        // Odysseas: This is a hack to use the new unified codepath for validate_transaction_ext_db
        // Effectively, we are applying the transaction in a clone of the currently running database
        // which is then used by the fork_db.
        // TODO: Remove this once we have a proper way to handle this.
        let mut ext_db = revm::db::WrapDatabaseRef(fork_db.clone());

        // Store assertions
        let tx_validation = assertion_executor
            .validate_transaction_ext_db(block, tx_env, &mut fork_db, &mut ext_db)
            .map_err(|e| format!("Assertion Executor Error: {e:#?}"))?;

        // if transaction execution reverted, bail
        if !tx_validation.result_and_state.result.is_success() {
            let decoded_error =
                decode_invalidated_assertion(&tx_validation.result_and_state.result);
            executor.console_log(ccx, &format!("Transaction reverted: {}", decoded_error.reason()));
            bail!("Transaction Reverted");
        }
        // else get information about the assertion execution
        let assertion_contract = tx_validation.assertions_executions.first().unwrap();
        let total_assertion_gas = tx_validation.total_assertions_gas();
        let total_assertions_ran = tx_validation.total_assertion_funcs_ran();
        let tx_gas_used = tx_validation.result_and_state.result.gas_used();
        let mut assertion_gas_message = format!(
            "Transaction gas cost: {tx_gas_used}\n  Total Assertion gas cost: {total_assertion_gas}\n  Total assertions ran: {total_assertions_ran}\n  Assertion Functions gas cost\n  "
        );

        // Format individual assertion function results
        for (fn_selector_index, assertion_fn) in
            assertion_contract.assertion_fns_results.iter().enumerate()
        {
            assertion_gas_message.push_str(&format!(
                "   └─ [selector {}:index {}] gas cost: {}\n",
                assertion_fn.id.fn_selector,
                fn_selector_index,
                assertion_fn.as_result().gas_used()
            ));
        }
        executor.console_log(ccx, &assertion_gas_message);

        if !tx_validation.is_valid() {
            let mut error_msg = format!("\n  {assertionContractLabel} Enforced Assertions:\n");
            // Collect failed assertions
            let reverted_assertions: HashMap<_, _> = assertion_contract
                .assertion_fns_results
                .iter()
                .enumerate()
                .filter(|(_, assertion_fn)| !assertion_fn.is_success())
                .map(|(fn_selector_index, assertion_fn)| {
                    let key = format!(
                        "[selector {}:index {}]",
                        assertion_fn.id.fn_selector, fn_selector_index
                    );
                    let revert = decode_invalidated_assertion(assertion_fn.as_result());
                    (key, revert)
                })
                .collect();

            // Format error messages
            for (key, revert) in reverted_assertions {
                error_msg.push_str(&format!(
                    "   └─ {} - Revert Reason: {} \n",
                    key,
                    revert.reason()
                ));
            }

            executor.console_log(ccx, &error_msg);
            bail!("Assertions Reverted");
        }
        Ok(Default::default())
    }
}

fn decode_invalidated_assertion(execution_result: &ExecutionResult) -> Revert {
    let result = execution_result;
    match result {
        ExecutionResult::Success{..} => Revert {
            reason: "Tried to decode invalidated assertion, but result was success. This is a bug in phoundry. Please report to the Phylax team.".to_string(),
        },
        ExecutionResult::Revert{output, ..} => {
            Revert::abi_decode(output, true)
                .unwrap_or(Revert::new(("Unknown Revert Reason".to_string(),)))
        },
        ExecutionResult::Halt{reason, ..} => Revert {
            reason: format!("Halt reason: {reason:#?}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Bytes;
    use revm::primitives::{HaltReason, Output, SuccessReason};

    #[test]
    fn test_decode_revert_error_success() {
        // Test case 1: When result is success
        let result = ExecutionResult::Success {
            gas_used: 0,
            gas_refunded: 0,
            logs: vec![],
            output: Output::Call(Bytes::new()),
            reason: SuccessReason::Return,
        };
        let revert = decode_invalidated_assertion(&result);
        assert_eq!(revert.reason(), "Tried to decode invalidated assertion, but result was success. This is a bug in phoundry. Please report to the Phylax team.");
    }

    #[test]
    fn test_decode_revert_error_revert() {
        let revert_reason = "Something is a bit fky wuky";
        let revert_output = Revert::new((revert_reason.to_string(),)).abi_encode();
        let result = ExecutionResult::Revert { output: revert_output.into(), gas_used: 0 };
        let revert = decode_invalidated_assertion(&result);
        assert_eq!(revert.reason(), revert_reason);
    }

    #[test]
    fn test_decode_revert_error_halt() {
        let halt_reason = HaltReason::CallTooDeep;
        let result = ExecutionResult::Halt { reason: halt_reason, gas_used: 0 };

        let revert = decode_invalidated_assertion(&result);
        assert_eq!(revert.reason(), "Halt reason: CallTooDeep");
    }
}
