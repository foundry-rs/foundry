use super::{
    InvariantFailures, InvariantFuzzError, InvariantMetrics, InvariantTest, InvariantTestRun,
    call_after_invariant_function, call_invariant_function, error::FailedInvariantCaseData,
};
use crate::executors::{Executor, RawCallResult};
use alloy_dyn_abi::JsonAbiExt;
use alloy_primitives::I256;
use eyre::Result;
use foundry_config::InvariantConfig;
use foundry_evm_core::utils::StateChangeset;
use foundry_evm_coverage::{HitMaps, SourceHitMaps};
use foundry_evm_fuzz::{
    BasicTxDetails, FuzzedCases,
    invariant::{FuzzRunIdentifiedContracts, InvariantContract},
};
use revm_inspectors::tracing::CallTraceArena;
use std::{borrow::Cow, collections::HashMap};

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
    pub line_coverage: Option<HitMaps>,
    /// The source coverage info collected during the invariant test runs.
    pub source_coverage: Option<SourceHitMaps>,
    /// Fuzzed selectors metrics collected during the invariant test runs.
    pub metrics: HashMap<String, InvariantMetrics>,
    /// Number of failed replays from persisted corpus.
    pub failed_corpus_replays: usize,
    /// For optimization mode (int256 return): the best (maximum) value achieved.
    /// None means standard invariant check mode.
    pub optimization_best_value: Option<I256>,
    /// For optimization mode: the call sequence that produced the best value.
    pub optimization_best_sequence: Vec<BasicTxDetails>,
}

/// Enriched results of an invariant run check.
///
/// Contains the success condition and call results of the last run
pub(crate) struct RichInvariantResults {
    pub(crate) can_continue: bool,
    pub(crate) call_result: Option<RawCallResult>,
}

impl RichInvariantResults {
    pub(crate) fn new(can_continue: bool, call_result: Option<RawCallResult>) -> Self {
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

    if let Some(fuzzer) = &executor.inspector().fuzzer
        && let Some(call_generator) = &fuzzer.call_generator
    {
        inner_sequence.extend(call_generator.last_sequence.read().iter().cloned());
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
///
/// For optimization mode (int256 return), tracks the max value but never fails on invariant.
/// For check mode, asserts the invariant and fails if broken.
pub(crate) fn can_continue(
    invariant_contract: &InvariantContract<'_>,
    invariant_test: &mut InvariantTest,
    invariant_run: &mut InvariantTestRun,
    invariant_config: &InvariantConfig,
    call_result: RawCallResult,
    state_changeset: &StateChangeset,
) -> Result<RichInvariantResults> {
    let mut call_results = None;
    let is_optimization = invariant_contract.is_optimization();

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

    if !call_result.reverted && handlers_succeeded() {
        if let Some(traces) = call_result.traces {
            invariant_run.run_traces.push(traces);
        }

        if is_optimization {
            // Optimization mode: call invariant and track max value, never fail.
            let (inv_result, success) = call_invariant_function(
                &invariant_run.executor,
                invariant_contract.address,
                invariant_contract.invariant_function.abi_encode_input(&[])?.into(),
            )?;
            if success
                && inv_result.result.len() >= 32
                && let Some(value) = I256::try_from_be_slice(&inv_result.result[..32])
            {
                invariant_test.update_optimization_value(value, &invariant_run.inputs);
            }
            call_results = Some(inv_result);
        } else {
            // Check mode: assert invariants and fail if broken.
            call_results = assert_invariants(
                invariant_contract,
                invariant_config,
                &invariant_test.targeted_contracts,
                &invariant_run.executor,
                &invariant_run.inputs,
                &mut invariant_test.test_data.failures,
            )?;
            if call_results.is_none() {
                return Ok(RichInvariantResults::new(false, None));
            }
        }
    } else {
        // Increase the amount of reverts.
        let invariant_data = &mut invariant_test.test_data;
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
        } else if call_result.reverted && !is_optimization {
            // If we don't fail test on revert then remove last reverted call from inputs.
            // In optimization mode, we keep reverted calls to preserve warp/roll values
            // for correct replay during shrinking.
            invariant_run.inputs.pop();
        }
    }
    Ok(RichInvariantResults::new(true, call_results))
}

/// Given the executor state, asserts conditions within `afterInvariant` function.
/// If call fails then the invariant test is considered failed.
pub(crate) fn assert_after_invariant(
    invariant_contract: &InvariantContract<'_>,
    invariant_test: &mut InvariantTest,
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
