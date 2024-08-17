use super::{
    call_after_invariant_function, call_invariant_function, error::FailedInvariantCaseData,
    InvariantFailures, InvariantFuzzError, InvariantTest, InvariantTestRun,
};
use crate::executors::{Executor, RawCallResult};
use alloy_dyn_abi::JsonAbiExt;
use eyre::Result;
use foundry_config::InvariantConfig;
use foundry_evm_core::utils::StateChangeset;
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::{
    invariant::{BasicTxDetails, FuzzRunIdentifiedContracts, InvariantContract},
    FuzzedCases,
};
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
    /// The coverage info collected during the invariant test runs.
    pub coverage: Option<HitMaps>,
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

    if let Some(fuzzer) = &executor.inspector().fuzzer {
        if let Some(call_generator) = &fuzzer.call_generator {
            inner_sequence.extend(call_generator.last_sequence.read().iter().cloned());
        }
    }

    let (call_result, success) = call_invariant_function(
        executor,
        invariant_contract.address,
        invariant_contract.invariant_function.abi_encode_input(&[])?.into(),
    )?;
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

/// Returns if invariant test can continue and last successful call result of the invariant test
/// function (if it can continue).
pub(crate) fn can_continue(
    invariant_contract: &InvariantContract<'_>,
    invariant_test: &InvariantTest,
    invariant_run: &mut InvariantTestRun,
    invariant_config: &InvariantConfig,
    call_result: RawCallResult,
    state_changeset: &StateChangeset,
) -> Result<RichInvariantResults> {
    let mut call_results = None;

    let handlers_succeeded = || {
        invariant_test.targeted_contracts.targets.lock().keys().all(|address| {
            invariant_run.executor.is_success(
                *address,
                false,
                Cow::Borrowed(state_changeset),
                false,
            )
        })
    };

    // Assert invariants if the call did not revert and the handlers did not fail.
    if !call_result.reverted && handlers_succeeded() {
        if let Some(traces) = call_result.traces {
            invariant_run.run_traces.push(traces);
        }

        call_results = assert_invariants(
            invariant_contract,
            invariant_config,
            &invariant_test.targeted_contracts,
            &invariant_run.executor,
            &invariant_run.inputs,
            &mut invariant_test.execution_data.borrow_mut().failures,
        )?;
        if call_results.is_none() {
            return Ok(RichInvariantResults::new(false, None));
        }
    } else {
        // Increase the amount of reverts.
        let mut invariant_data = invariant_test.execution_data.borrow_mut();
        invariant_data.failures.reverts += 1;
        // If fail on revert is set, we must return immediately.
        if invariant_config.fail_on_revert {
            let case_data = FailedInvariantCaseData::new(
                invariant_contract,
                invariant_config,
                &invariant_test.targeted_contracts,
                &invariant_run.inputs,
                call_result,
                &[],
            );
            invariant_data.failures.revert_reason = Some(case_data.revert_reason.clone());
            invariant_data.failures.error = Some(InvariantFuzzError::Revert(case_data));

            return Ok(RichInvariantResults::new(false, None));
        } else if call_result.reverted {
            // If we don't fail test on revert then remove last reverted call from inputs.
            // This improves shrinking performance as irrelevant calls won't be checked again.
            invariant_run.inputs.pop();
        }
    }
    Ok(RichInvariantResults::new(true, call_results))
}

/// Given the executor state, asserts conditions within `afterInvariant` function.
/// If call fails then the invariant test is considered failed.
pub(crate) fn assert_after_invariant(
    invariant_contract: &InvariantContract<'_>,
    invariant_test: &InvariantTest,
    invariant_run: &InvariantTestRun,
    invariant_config: &InvariantConfig,
) -> Result<bool> {
    let (call_result, success) =
        call_after_invariant_function(&invariant_run.executor, invariant_contract.address)?;
    // Fail the test case if `afterInvariant` doesn't succeed.
    if !success {
        let case_data = FailedInvariantCaseData::new(
            invariant_contract,
            invariant_config,
            &invariant_test.targeted_contracts,
            &invariant_run.inputs,
            call_result,
            &[],
        );
        invariant_test.set_error(InvariantFuzzError::BrokenInvariant(case_data));
    }
    Ok(success)
}
