use crate::{inspector::Ecx, Cheatcode, Cheatcodes, CheatcodesExecutor, CheatsCtxt, Result, Vm::*};
use alloy_primitives::{Bytes, FixedBytes, TxKind};
use alloy_sol_types::{Revert, SolError};
use assertion_executor::{
    db::{fork_db::ForkDb, DatabaseCommit, DatabaseRef},
    primitives::{
        AccountInfo, Address, AssertionFunctionExecutionResult, Bytecode, ExecutionResult, TxEnv,
        B256, U256,
    },
    store::{AssertionState, AssertionStore},
    ExecutorConfig,
};

use foundry_evm_core::backend::{DatabaseError, DatabaseExt};
use revm::context_interface::{ContextTr, JournalTr};
use std::{
    cmp::max,
    collections::HashSet,
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

    fn basic_ref(
        &self,
        address: Address,
    ) -> Result<Option<AccountInfo>, <Self as DatabaseRef>::Error> {
        self.db.lock().unwrap().basic(address)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, <Self as DatabaseRef>::Error> {
        self.db.lock().unwrap().code_by_hash(code_hash)
    }

    fn storage_ref(
        &self,
        address: Address,
        index: U256,
    ) -> Result<U256, <Self as DatabaseRef>::Error> {
        self.db.lock().unwrap().storage(address, index)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, <Self as DatabaseRef>::Error> {
        self.db.lock().unwrap().block_hash(number)
    }
}

impl Cheatcode for assertionCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { adopter, createData: create_data, fnSelector: fn_selector } = self;

        ensure!(
            ccx.state.assertion.is_none(),
            "you must call another function prior to setting another assertion"
        );
        let assertion = Assertion {
            adopter: *adopter,
            create_data: create_data.to_vec(),
            fn_selector: *fn_selector,
            depth: ccx.ecx.journaled_state.depth(),
        };

        ccx.state.assertion = Some(assertion);
        Ok(Default::default())
    }
}
#[derive(Debug, Clone)]
pub struct Assertion {
    pub adopter: Address,
    pub create_data: Vec<u8>,
    pub fn_selector: FixedBytes<4>,
    pub depth: usize,
}

pub struct TxAttributes {
    pub value: U256,
    pub data: Bytes,
    pub caller: Address,
    pub kind: TxKind,
}

/// Used to handle assertion execution in inspector in calls after the cheatcode was called.
pub fn execute_assertion(
    assertion: &Assertion,
    tx_attributes: TxAttributes,
    ecx: Ecx,
    executor: &mut dyn CheatcodesExecutor,
    cheats: &mut Cheatcodes,
) -> Result<(), crate::Error> {
    let spec_id = ecx.cfg.spec;
    let block = ecx.block.clone();
    let state = ecx.journaled_state.state.clone();
    let chain_id = ecx.cfg.chain_id;

    let nonce = ecx.db().basic(tx_attributes.caller).unwrap_or_default().unwrap_or_default().nonce;
    // Setup assertion database
    let db = ThreadSafeDb::new(ecx.db());

    // Prepare assertion store

    let config = ExecutorConfig { spec_id, chain_id, assertion_gas_limit: 100_000 };

    let store = AssertionStore::new_ephemeral().expect("Failed to create assertion store");

    let mut assertion_state =
        AssertionState::new_active(assertion.create_data.clone().into(), &config)
            .expect("Failed to create assertion state");

    let mut trigger_types_to_remove = Vec::new();
    // Filter triggers for one fn selector
    for (trigger_type, fn_selectors) in assertion_state.trigger_recorder.triggers.iter_mut() {
        if fn_selectors.contains(&assertion.fn_selector) {
            *fn_selectors = HashSet::from_iter([assertion.fn_selector]);
        } else {
            trigger_types_to_remove.push(trigger_type.clone());
        }
    }
    for trigger_type in trigger_types_to_remove {
        assertion_state.trigger_recorder.triggers.remove(&trigger_type);
    }

    store.insert(assertion.adopter, assertion_state).expect("Failed to store assertions");
    let tx_env = TxEnv {
        caller: tx_attributes.caller,
        gas_limit: block.gas_limit.try_into().unwrap_or(u64::MAX),
        gas_price: block.basefee.into(),
        chain_id: Some(chain_id),
        value: tx_attributes.value,
        data: tx_attributes.data,
        kind: tx_attributes.kind,
        nonce,
        ..Default::default()
    };

    let mut assertion_executor = config.build(store);

    // Commit current journal state so that it is available for assertions and
    // triggering tx
    let mut fork_db = ForkDb::new(db.clone());
    fork_db.commit(state);

    // Odysseas: This is a hack to use the new unified codepath for validate_transaction_ext_db
    // Effectively, we are applying the transaction in a clone of the currently running database
    // which is then used by the fork_db.
    // TODO: Remove this once we have a proper way to handle this.
    let mut ext_db = revm::database::WrapDatabaseRef(fork_db.clone());

    // Store assertions
    let tx_validation = assertion_executor
        .validate_transaction_ext_db(block.clone(), tx_env.clone(), &mut fork_db, &mut ext_db)
        .map_err(|e| format!("Assertion Executor Error: {e:#?}"))?;

    let mut inspector = executor.get_inspector(cheats);
    // if transaction execution reverted, log the revert reason
    if !tx_validation.result_and_state.result.is_success() {
        inspector.console_log(&format!(
            "Transaction reverted: {}",
            decode_invalidated_assertion(&tx_validation.result_and_state.result).reason()
        ));
    }

    // else get information about the assertion execution
    let total_assertion_gas = tx_validation.total_assertions_gas();
    let total_assertions_ran = tx_validation.total_assertion_funcs_ran();
    let tx_gas_used = tx_validation.result_and_state.result.gas_used();

    if total_assertions_ran != 1 {
        // If assertions were not executed, we need to update expect revert depth to
        // allow for matching on this revert condition, as we will not execute against
        // test evm in this case.
        ecx.journaled_state.inner.checkpoint();

        std::mem::drop(inspector);
        if let Some(expected) = &mut cheats.expected_revert {
            expected.max_depth = max(ecx.journaled_state.depth(), expected.max_depth);
        }
        bail!("Expected 1 assertion to be executed, but {total_assertions_ran} were executed.");
    }

    //Expect is safe because we validate above that 1 assertion was ran.
    let assertion_contract = tx_validation
        .assertions_executions
        .first()
        .expect("Expected 1 assertion to be executed, but got 0");

    let assertion_fn_result = assertion_contract
        .assertion_fns_results
        .first()
        .expect("Expected 1 assertion to be executed, but got 0");

    if !assertion_fn_result.console_logs.is_empty() {
        inspector.console_log("Assertion function logs: ");
        for log in assertion_fn_result.console_logs.iter() {
            inspector.console_log(log);
        }
    }

    let assertion_gas_message = format!(
        "Transaction gas cost: {tx_gas_used}\n  Assertion gas cost: {total_assertion_gas}\n  "
    );
    inspector.console_log(&assertion_gas_message);

    // Drop the inspector to avoid borrow checker issues
    std::mem::drop(inspector);

    if !tx_validation.is_valid() {
        // If invalidated, we don't execute against test evm, so we must update expected depth
        // for expect revert cheatcode.
        ecx.journaled_state.inner.checkpoint();

        if let Some(expected) = &mut cheats.expected_revert {
            expected.max_depth = max(ecx.journaled_state.depth(), expected.max_depth);
        }

        let mut inspector = executor.get_inspector(cheats);
        match &assertion_fn_result.result {
            AssertionFunctionExecutionResult::AssertionContractDeployFailure(result) => {
                inspector.console_log(&format!(
                    "Assertion contract deploy failed: {}",
                    decode_invalidated_assertion(&result).reason()
                ));
                let output = result.output().unwrap_or_default();
                return Err(crate::Error::from(output.clone()));
            }
            AssertionFunctionExecutionResult::AssertionExecutionResult(result) => {
                inspector.console_log(&format!(
                    "Assertion function reverted: {}",
                    decode_invalidated_assertion(&result).reason()
                ));
                let output = result.output().unwrap_or_default();
                return Err(crate::Error::from(output.clone()));
            }
        }
    }
    Ok(())
}

fn decode_invalidated_assertion(execution_result: &ExecutionResult) -> Revert {
    let result = execution_result;
    match result {
        ExecutionResult::Success{..} => Revert {
            reason: "Tried to decode invalidated assertion, but result was success. This is a bug in phoundry. Please report to the Phylax team.".to_string(),
        },
        ExecutionResult::Revert{output, ..} => {
            Revert::abi_decode(output)
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
    use assertion_executor::primitives::HaltReason;
    use revm::context::result::{Output, SuccessReason};

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
