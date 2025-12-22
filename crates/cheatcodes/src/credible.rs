use crate::{Cheatcode, Cheatcodes, CheatcodesExecutor, CheatsCtxt, Result, Vm::*, inspector::Ecx};
use alloy_primitives::{Bytes, FixedBytes, TxKind};
use assertion_executor::{
    ExecutorConfig,
    db::{DatabaseCommit, DatabaseRef, fork_db::ForkDb},
    primitives::{
        AccountInfo, Address, AssertionFunctionExecutionResult, B256, Bytecode, ExecutionResult,
        TxEnv, U256,
    },
    store::{AssertionState, AssertionStore},
};
use foundry_evm_core::{ContextExt, decode::RevertDecoder};
use foundry_fork_db::DatabaseError;

use foundry_evm_core::backend::DatabaseExt;
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

/// Maximum gas allowed for assertion execution (300k gas).
/// Assertions exceeding this limit will cause the test to fail.
const ASSERTION_GAS_LIMIT: u64 = 300_000;

/// Checks if the assertion gas usage is within the allowed limit.
/// Returns a detailed log message if the limit is exceeded, None otherwise.
fn check_assertion_gas_limit(gas_used: u64) -> Option<String> {
    if gas_used > ASSERTION_GAS_LIMIT {
        let over_by = gas_used - ASSERTION_GAS_LIMIT;
        let over_percent = (over_by as f64 / ASSERTION_GAS_LIMIT as f64) * 100.0;
        Some(format!(
            "Assertion used {gas_used} gas, exceeding limit of {ASSERTION_GAS_LIMIT} by {over_by} ({over_percent:.1}% over)"
        ))
    } else {
        None
    }
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

    let (db, journal, _) = ecx.as_db_env_and_journal();
    let nonce = journal
        .load_account(db, tx_attributes.caller)
        .map(|acc| acc.info.nonce)
        .unwrap_or(0);
    // Setup assertion database
    let db = ThreadSafeDb::new(*ecx.db_mut());

    // Prepare assertion store

    let config =
        ExecutorConfig { spec_id: spec_id.into(), chain_id, assertion_gas_limit: u64::MAX };

    let store = AssertionStore::new_ephemeral().expect("Failed to create assertion store");

    if assertion.create_data.is_empty() {
        bail!("Assertion bytecode is empty");
    }

    let mut assertion_state =
        AssertionState::new_active(&Bytes::from_iter(assertion.create_data.clone()), &config)
            .expect("Failed to create assertion state");

    // Filter triggers to only keep those matching our fn_selector
    assertion_state.trigger_recorder.triggers.retain(|_, fn_selectors| {
        if fn_selectors.contains(&assertion.fn_selector) {
            *fn_selectors = HashSet::from_iter([assertion.fn_selector]);
            true
        } else {
            false
        }
    });

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

    // Execute assertion validation
    let tx_validation = assertion_executor
        .validate_transaction_ext_db(block, tx_env, &mut fork_db, &mut ext_db)
        .map_err(|e| format!("Assertion Executor Error: {e:#?}"))?;

    let mut inspector = executor.get_inspector(cheats);
    // if transaction execution reverted, log the revert reason
    if !tx_validation.result_and_state.result.is_success() {
        inspector.console_log(&format!(
            "Mock Transaction Revert Reason: {}",
            decode_invalidated_assertion(&tx_validation.result_and_state.result)
        ));
        bail!("Mock Transaction Reverted");
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

        // Drop inspector first to release the borrow on cheats
        std::mem::drop(inspector);
        if let Some(expected) = &mut cheats.expected_revert {
            expected.max_depth = max(ecx.journaled_state.depth(), expected.max_depth);
        }
        // Get a new inspector for logging
        let mut inspector = executor.get_inspector(cheats);
        inspector.console_log(&format!(
            "Expected 1 assertion fn to be executed, but {total_assertions_ran} were executed."
        ));
        bail!("Assertion Fn number mismatch");
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

    inspector.console_log(&format!(
        "Transaction gas cost: {tx_gas_used}\n  Assertion gas cost: {total_assertion_gas}"
    ));

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
        let (msg, result) = match &assertion_fn_result.result {
            AssertionFunctionExecutionResult::AssertionContractDeployFailure(r) => {
                ("Assertion contract deploy failed", r)
            }
            AssertionFunctionExecutionResult::AssertionExecutionResult(r) => {
                ("Assertion function reverted", r)
            }
        };
        inspector.console_log(&format!("{msg}: {}", decode_invalidated_assertion(result)));
        return Err(crate::Error::from(result.output().unwrap_or_default().clone()));
    }

    if let Some(log_msg) = check_assertion_gas_limit(total_assertion_gas) {
        let mut inspector = executor.get_inspector(cheats);
        inspector.console_log(&log_msg);
        bail!("Assertion exceeded gas limit");
    }

    Ok(())
}

/// Decodes revert data from an assertion execution result.
/// Uses foundry's RevertDecoder to handle all revert types:
/// - Error(string) from revert()/require()
/// - Panic(uint256) from assert()/overflow/etc
/// - Custom errors
/// - Raw bytes as fallback
fn decode_invalidated_assertion(result: &ExecutionResult) -> String {
    match result {
        ExecutionResult::Success { .. } => {
            "Tried to decode invalidated assertion, but result was success. \
             This is a bug in phoundry. Please report to the Phylax team."
                .to_string()
        }
        ExecutionResult::Revert { output, .. } => RevertDecoder::default().decode(output, None),
        ExecutionResult::Halt { reason, .. } => {
            format!("Halt reason: {reason:#?}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_sol_types::{Revert, SolError};
    use assertion_executor::primitives::HaltReason;
    use revm::context_interface::result::{Output, SuccessReason};

    #[test]
    fn test_decode_revert_error_success() {
        let result = ExecutionResult::Success {
            gas_used: 0,
            gas_refunded: 0,
            logs: vec![],
            output: Output::Call(Bytes::new()),
            reason: SuccessReason::Return,
        };
        let decoded = decode_invalidated_assertion(&result);
        assert!(decoded.contains("bug in phoundry"));
    }

    #[test]
    fn test_decode_revert_error_revert() {
        let revert_reason = "Something is a bit fky wuky";
        let revert_output = Revert::new((revert_reason.to_string(),)).abi_encode();
        let result = ExecutionResult::Revert { output: revert_output.into(), gas_used: 0 };
        let decoded = decode_invalidated_assertion(&result);
        assert_eq!(decoded, revert_reason);
    }

    #[test]
    fn test_decode_revert_panic() {
        // Panic(uint256) with code 0x01 (assertion failed)
        let panic_output = alloy_sol_types::Panic::from(0x01).abi_encode();
        let result = ExecutionResult::Revert { output: panic_output.into(), gas_used: 0 };
        let decoded = decode_invalidated_assertion(&result);
        assert!(decoded.contains("Panic") || decoded.contains("panic"));
    }

    #[test]
    fn test_decode_revert_raw_bytes() {
        // Raw bytes that don't match any known format
        let raw_bytes = vec![0xde, 0xad, 0xbe, 0xef];
        let result = ExecutionResult::Revert { output: raw_bytes.into(), gas_used: 0 };
        let decoded = decode_invalidated_assertion(&result);
        // Should contain hex representation
        assert!(decoded.contains("deadbeef") || decoded.contains("custom error"));
    }

    #[test]
    fn test_decode_revert_error_halt() {
        let halt_reason = HaltReason::CallTooDeep;
        let result = ExecutionResult::Halt { reason: halt_reason, gas_used: 0 };
        let decoded = decode_invalidated_assertion(&result);
        assert_eq!(decoded, "Halt reason: CallTooDeep");
    }

    #[test]
    fn test_assertion_gas_limit_constant() {
        // Ensure the gas limit is set to the expected value (300k)
        assert_eq!(ASSERTION_GAS_LIMIT, 300_000);
    }

    #[test]
    fn test_check_gas_limit_under() {
        // Gas usage under limit should return None
        assert!(check_assertion_gas_limit(100_000).is_none());
        assert!(check_assertion_gas_limit(299_999).is_none());
    }

    #[test]
    fn test_check_gas_limit_exact() {
        // Gas usage exactly at limit should return None
        assert!(check_assertion_gas_limit(300_000).is_none());
    }

    #[test]
    fn test_check_gas_limit_over() {
        // Gas usage over limit should return error message with details
        let result = check_assertion_gas_limit(450_000);
        assert!(result.is_some());
        let msg = result.unwrap();
        // Should contain: gas used, limit, absolute over, percentage
        assert!(msg.contains("450000"), "should contain gas used");
        assert!(msg.contains("300000"), "should contain limit");
        assert!(msg.contains("150000"), "should contain absolute over amount");
        assert!(msg.contains("50.0%"), "should contain percentage over");
    }

    #[test]
    fn test_check_gas_limit_over_small() {
        // Just 1 gas over the limit
        let result = check_assertion_gas_limit(300_001);
        assert!(result.is_some());
        let msg = result.unwrap();
        assert!(msg.contains("300001"));
        assert!(msg.contains("by 1"));
        assert!(msg.contains("0.0%")); // 1/300000 ≈ 0.0003%
    }

    #[test]
    fn test_check_gas_limit_zero() {
        // Zero gas should be fine
        assert!(check_assertion_gas_limit(0).is_none());
    }

    #[test]
    fn test_check_gas_limit_max() {
        // Max u64 should definitely exceed
        let result = check_assertion_gas_limit(u64::MAX);
        assert!(result.is_some());
    }
}
