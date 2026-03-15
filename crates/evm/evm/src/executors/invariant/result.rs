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
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::{
    BasicTxDetails, FuzzedCases,
    invariant::{FuzzRunIdentifiedContracts, InvariantContract},
};
use revm::interpreter::InstructionResult;
use revm_inspectors::tracing::CallTraceArena;
use std::{borrow::Cow, collections::HashMap};

/// The outcome of an invariant fuzz test
#[derive(Debug)]
pub struct InvariantFuzzTestResult {
    pub error: Option<InvariantFuzzError>,
    /// Distinct handler-level assertion failures observed during the campaign.
    pub assertion_failures: Vec<String>,
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

/// Returns true if this call failed due to a Solidity assert:
/// - Panic(0x01), or
/// - legacy invalid opcode assert behavior.
pub(crate) fn is_assertion_failure(call_result: &RawCallResult) -> bool {
    if !call_result.reverted {
        return false;
    }

    is_assert_panic(call_result.result.as_ref())
        || matches!(call_result.exit_reason, Some(InstructionResult::InvalidFEOpcode))
}

fn is_assert_panic(data: &[u8]) -> bool {
    const PANIC_SELECTOR: [u8; 4] = [0x4e, 0x48, 0x7b, 0x71];
    if data.len() < 36 || data[..4] != PANIC_SELECTOR {
        return false;
    }

    let panic_code = &data[4..36];
    panic_code[..31].iter().all(|byte| *byte == 0) && panic_code[31] == 0x01
}

fn failing_handler_name(
    invariant_test: &InvariantTest,
    invariant_run: &InvariantTestRun,
) -> Option<String> {
    let last_input = invariant_run.inputs.last()?;
    let metric_key =
        invariant_test.targeted_contracts.targets.lock().fuzzed_metric_key(last_input)?;
    Some(metric_key.rsplit('.').next().unwrap_or(metric_key.as_str()).to_string())
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
        let is_assert_failure = is_assertion_failure(&call_result);
        let should_fail_on_assert = invariant_config.fail_on_assert && is_assert_failure;
        let failing_handler = if should_fail_on_assert {
            failing_handler_name(invariant_test, invariant_run)
        } else {
            None
        };
        // Increase the amount of reverts.
        let invariant_data = &mut invariant_test.test_data;
        invariant_data.failures.reverts += 1;

        // In fail-on-assert mode, keep exploring and accumulate unique assertion failures.
        if should_fail_on_assert {
            let case_data = FailedInvariantCaseData::new(
                invariant_contract,
                invariant_config,
                &invariant_test.targeted_contracts,
                &invariant_run.inputs,
                call_result,
                &[],
            )
            .with_failing_handler(failing_handler);
            invariant_data.failures.revert_reason = Some(case_data.revert_reason.clone());
            invariant_data.failures.record_assertion_failure(case_data);
            if !is_optimization {
                // Keep shrinking/replay coherent by discarding reverted calls in check mode.
                invariant_run.inputs.pop();
            }
            return Ok(RichInvariantResults::new(true, None));
        } else if invariant_config.fail_on_revert {
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Bytes;

    fn panic_payload(code: u8) -> Bytes {
        let mut payload = vec![0_u8; 36];
        payload[..4].copy_from_slice(&[0x4e, 0x48, 0x7b, 0x71]);
        payload[35] = code;
        payload.into()
    }

    #[test]
    fn detects_assert_panic_code() {
        let call_result =
            RawCallResult { reverted: true, result: panic_payload(0x01), ..Default::default() };
        assert!(is_assertion_failure(&call_result));
    }

    #[test]
    fn ignores_non_assert_panic_code() {
        let call_result =
            RawCallResult { reverted: true, result: panic_payload(0x11), ..Default::default() };
        assert!(!is_assertion_failure(&call_result));
    }

    #[test]
    fn detects_legacy_invalid_opcode_assert() {
        let call_result = RawCallResult {
            reverted: true,
            exit_reason: Some(InstructionResult::InvalidFEOpcode),
            ..Default::default()
        };
        assert!(is_assertion_failure(&call_result));
    }
}
