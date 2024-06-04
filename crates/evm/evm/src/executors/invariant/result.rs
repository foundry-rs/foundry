use super::{error::FailedInvariantCaseData, InvariantFailures, InvariantFuzzError};
use crate::executors::{Executor, RawCallResult};
use alloy_dyn_abi::JsonAbiExt;
use eyre::Result;
use foundry_config::InvariantConfig;
use foundry_evm_core::{constants::CALLER, utils::StateChangeset};
use foundry_evm_fuzz::{
    invariant::{BasicTxDetails, FuzzRunIdentifiedContracts, InvariantContract},
    FuzzedCases,
};
use revm::primitives::U256;
use revm_inspectors::tracing::CallTraceArena;
use std::borrow::Cow;

/// The outcome of an invariant fuzz test
#[derive(Debug)]
pub struct InvariantFuzzTestResult {
    pub error: Option<InvariantFuzzError>,
    /// Every successful fuzz test case
    pub cases: Vec<FuzzedCases>,
    /// Number of reverted fuzz calls
    pub reverts: usize,
    /// The entire inputs of the last run of the invariant campaign, used for
    /// replaying the run for collecting traces.
    pub last_run_inputs: Vec<BasicTxDetails>,
    /// Additional traces used for gas report construction.
    pub gas_report_traces: Vec<Vec<CallTraceArena>>,
}

/// Enriched results of an invariant run check.
///
/// Contains the success condition and call results of the last run
pub(crate) struct RichInvariantResults {
    pub(crate) can_continue: bool,
    pub(crate) call_result: Option<RawCallResult>,
}

impl RichInvariantResults {
    fn new(can_continue: bool, call_result: Option<RawCallResult>) -> Self {
        Self { can_continue, call_result }
    }
}

/// Given the executor state, asserts that no invariant has been broken. Otherwise, it fills the
/// external `invariant_failures.failed_invariant` map and returns a generic error.
/// Either returns the call result if successful, or nothing if there was an error.
pub(crate) fn assert_invariants(
    invariant_contract: &InvariantContract<'_>,
    invariant_config: &InvariantConfig,
    targeted_contracts: &FuzzRunIdentifiedContracts,
    executor: &Executor,
    calldata: &[BasicTxDetails],
    invariant_failures: &mut InvariantFailures,
) -> Result<Option<RawCallResult>> {
    let mut inner_sequence = vec![];

    if let Some(fuzzer) = &executor.inspector.fuzzer {
        if let Some(call_generator) = &fuzzer.call_generator {
            inner_sequence.extend(call_generator.last_sequence.read().iter().cloned());
        }
    }

    let func = invariant_contract.invariant_function;
    let mut call_result = executor.call_raw(
        CALLER,
        invariant_contract.address,
        func.abi_encode_input(&[])?.into(),
        U256::ZERO,
    )?;

    let success = executor.is_raw_call_success(
        invariant_contract.address,
        Cow::Owned(call_result.state_changeset.take().unwrap()),
        &call_result,
        false,
    );
    if !success {
        // We only care about invariants which we haven't broken yet.
        if invariant_failures.error.is_none() {
            let case_data = FailedInvariantCaseData::new(
                invariant_contract,
                invariant_config,
                targeted_contracts,
                calldata,
                call_result,
                &inner_sequence,
            );
            invariant_failures.error = Some(InvariantFuzzError::BrokenInvariant(case_data));
            return Ok(None);
        }
    }

    Ok(Some(call_result))
}

/// Verifies that the invariant run execution can continue.
/// Returns the mapping of (Invariant Function Name -> Call Result, Logs, Traces) if invariants were
/// asserted.
#[allow(clippy::too_many_arguments)]
pub(crate) fn can_continue(
    invariant_contract: &InvariantContract<'_>,
    invariant_config: &InvariantConfig,
    call_result: RawCallResult,
    executor: &Executor,
    calldata: &[BasicTxDetails],
    failures: &mut InvariantFailures,
    targeted_contracts: &FuzzRunIdentifiedContracts,
    state_changeset: &StateChangeset,
    run_traces: &mut Vec<CallTraceArena>,
) -> Result<RichInvariantResults> {
    let mut call_results = None;

    let handlers_succeeded = || {
        targeted_contracts.targets.lock().iter().all(|(address, ..)| {
            executor.is_success(*address, false, Cow::Borrowed(state_changeset), false)
        })
    };

    // Assert invariants if the call did not revert and the handlers did not fail.
    if !call_result.reverted && handlers_succeeded() {
        if let Some(traces) = call_result.traces {
            run_traces.push(traces);
        }

        call_results = assert_invariants(
            invariant_contract,
            invariant_config,
            targeted_contracts,
            executor,
            calldata,
            failures,
        )?;
        if call_results.is_none() {
            return Ok(RichInvariantResults::new(false, None));
        }
    } else {
        // Increase the amount of reverts.
        failures.reverts += 1;
        // If fail on revert is set, we must return immediately.
        if invariant_config.fail_on_revert {
            let case_data = FailedInvariantCaseData::new(
                invariant_contract,
                invariant_config,
                targeted_contracts,
                calldata,
                call_result,
                &[],
            );
            failures.revert_reason = Some(case_data.revert_reason.clone());
            let error = InvariantFuzzError::Revert(case_data);
            failures.error = Some(error);

            return Ok(RichInvariantResults::new(false, None));
        }
    }
    Ok(RichInvariantResults::new(true, call_results))
}
