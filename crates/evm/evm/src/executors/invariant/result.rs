use super::{
    InvariantFailures, InvariantFuzzError, InvariantMetrics, InvariantTest, InvariantTestRun,
    call_after_invariant_function, call_invariant_function, error::FailedInvariantCaseData,
};
use crate::executors::{Executor, RawCallResult};
use alloy_dyn_abi::JsonAbiExt;
use eyre::Result;
use foundry_config::InvariantConfig;
use foundry_evm_core::utils::StateChangeset;
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::{
    BasicTxDetails, FuzzedCases,
    invariant::{FuzzRunIdentifiedContracts, InvariantContract},
};
use revm_inspectors::tracing::CallTraceArena;
use std::{borrow::Cow, collections::HashMap};

/// The outcome of an invariant fuzz test
#[derive(Debug)]
pub struct InvariantFuzzTestResult {
    /// Errors recorded per invariant.
    pub errors: HashMap<String, InvariantFuzzError>,
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
    /// NUmber of failed replays from persisted corpus.
    pub failed_corpus_replays: usize,
}

/// Given the executor state, asserts that no invariant has been broken. Otherwise, it fills the
/// external `invariant_failures.failed_invariant` map and returns a generic error.
/// Either returns the call result if successful, or nothing if there was an error.
pub(crate) fn invariant_preflight_check(
    invariant_contract: &InvariantContract<'_>,
    invariant_config: &InvariantConfig,
    targeted_contracts: &FuzzRunIdentifiedContracts,
    executor: &Executor,
    calldata: &[BasicTxDetails],
    invariant_failures: &mut InvariantFailures,
) -> Result<()> {
    let (call_result, success) = call_invariant_function(
        executor,
        invariant_contract.address,
        invariant_contract.invariant_fn.abi_encode_input(&[])?.into(),
    )?;
    if !success {
        // We only care about invariants which we haven't broken yet.
        invariant_failures.record_failure(
            invariant_contract.invariant_fn,
            InvariantFuzzError::BrokenInvariant(FailedInvariantCaseData::new(
                invariant_contract,
                invariant_config.shrink_run_limit,
                invariant_config.fail_on_revert,
                targeted_contracts,
                calldata,
                &call_result,
                &invariant_inner_sequence(executor),
            )),
        );
    }

    Ok(())
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
) -> Result<()> {
    let inner_sequence = invariant_inner_sequence(executor);
    // We only care about invariants which we haven't broken yet.
    for (invariant, fail_on_revert) in &invariant_contract.invariant_fns {
        // We only care about invariants which we haven't broken yet.
        if invariant_failures.has_failure(invariant) {
            continue;
        }

        let (call_result, success) = call_invariant_function(
            executor,
            invariant_contract.address,
            invariant.abi_encode_input(&[])?.into(),
        )?;
        if !success {
            invariant_failures.record_failure(
                invariant,
                InvariantFuzzError::BrokenInvariant(FailedInvariantCaseData::new(
                    invariant_contract,
                    invariant_config.shrink_run_limit,
                    *fail_on_revert,
                    targeted_contracts,
                    calldata,
                    &call_result,
                    &inner_sequence,
                )),
            );
        }
    }

    Ok(())
}

/// Helper function to initialize invariant inner sequence.
fn invariant_inner_sequence(executor: &Executor) -> Vec<Option<BasicTxDetails>> {
    let mut seq = vec![];
    if let Some(fuzzer) = &executor.inspector().fuzzer
        && let Some(call_generator) = &fuzzer.call_generator
    {
        seq.extend(call_generator.last_sequence.read().iter().cloned());
    }
    seq
}

/// Returns if invariant test can continue and last successful call result of the invariant test
/// function (if it can continue).
pub(crate) fn can_continue(
    invariant_contract: &InvariantContract<'_>,
    invariant_test: &mut InvariantTest,
    invariant_run: &mut InvariantTestRun,
    invariant_config: &InvariantConfig,
    call_result: RawCallResult,
    state_changeset: &StateChangeset,
) -> Result<bool> {
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

    let failures = &mut invariant_test.test_data.failures;
    // Assert invariants if the call did not revert and the handlers did not fail.
    if !call_result.reverted && handlers_succeeded() {
        if let Some(traces) = call_result.traces {
            invariant_run.run_traces.push(traces);
        }
        assert_invariants(
            invariant_contract,
            invariant_config,
            &invariant_test.targeted_contracts,
            &invariant_run.executor,
            &invariant_run.inputs,
            failures,
        )?;
    } else {
        // Increase the amount of reverts.
        failures.reverts += 1;
        // If fail on revert is set, record invariant failure.
        for (invariant, fail_on_revert) in &invariant_contract.invariant_fns {
            if *fail_on_revert {
                let case_data = FailedInvariantCaseData::new(
                    invariant_contract,
                    invariant_config.shrink_run_limit,
                    *fail_on_revert,
                    &invariant_test.targeted_contracts,
                    &invariant_run.inputs,
                    &call_result,
                    &[],
                );
                failures
                    .errors
                    .insert(invariant.name.clone(), InvariantFuzzError::Revert(case_data));
            }
        }
        // Remove last reverted call from inputs.
        // This improves shrinking performance as irrelevant calls won't be checked again.
        invariant_run.inputs.pop();
    }
    // Stop execution if all invariants are broken.
    Ok(failures.can_continue(invariant_contract.invariant_fns.len()))
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
            invariant_config.shrink_run_limit,
            invariant_config.fail_on_revert,
            &invariant_test.targeted_contracts,
            &invariant_run.inputs,
            &call_result,
            &[],
        );
        invariant_test.set_error(
            invariant_contract.invariant_fn,
            InvariantFuzzError::BrokenInvariant(case_data),
        );
    }
    Ok(success)
}
