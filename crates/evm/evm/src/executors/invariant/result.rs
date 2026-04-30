use super::{
    InvariantFailures, InvariantFuzzError, InvariantMetrics, InvariantTest, InvariantTestRun,
    call_after_invariant_function, call_invariant_function,
    error::{FailedInvariantCaseData, HandlerAssertionFailure, handler_edge_fingerprint},
};
use crate::executors::{Executor, RawCallResult};
use alloy_dyn_abi::JsonAbiExt;
use alloy_primitives::{Address, B256, I256, Selector};
use alloy_sol_types::{Panic, PanicKind, Revert, SolError, SolInterface};
use eyre::Result;
use foundry_config::InvariantConfig;
use foundry_evm_core::{
    abi::Vm,
    constants::CHEATCODE_ADDRESS,
    decode::{ASSERTION_FAILED_PREFIX, decode_console_log},
    evm::FoundryEvmNetwork,
    utils::StateChangeset,
};
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
    /// Errors recorded per invariant.
    pub errors: HashMap<String, InvariantFuzzError>,
    /// Handler-side assertion bugs discovered during the campaign, keyed by the
    /// `(reverter, selector)` site of the asserting call. These are bugs in fuzzed handler
    /// functions, distinct from invariant predicate violations; the same handler function
    /// asserting via different code paths counts as a single bug.
    pub handler_errors: HashMap<(Address, Selector), HandlerAssertionFailure>,
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

/// Given the executor state, asserts that no invariant has been broken. Otherwise, it fills the
/// external `invariant_failures.failed_invariant` map and returns a generic error.
/// Either returns the call result if successful, or nothing if there was an error.
pub(crate) fn invariant_preflight_check<FEN: FoundryEvmNetwork>(
    invariant_contract: &InvariantContract<'_>,
    invariant_config: &InvariantConfig,
    targeted_contracts: &FuzzRunIdentifiedContracts,
    executor: &Executor<FEN>,
    calldata: &[BasicTxDetails],
    invariant_failures: &mut InvariantFailures,
) -> Result<()> {
    assert_invariants(
        invariant_contract,
        invariant_config,
        targeted_contracts,
        executor,
        calldata,
        invariant_failures,
    )
}

/// Returns true if this call failed due to a Solidity assertion:
/// - `Panic(0x01)`, or
/// - legacy invalid opcode assert behavior.
pub(crate) fn is_assertion_failure<FEN: FoundryEvmNetwork>(
    call_result: &RawCallResult<FEN>,
) -> bool {
    if !call_result.reverted {
        return false;
    }

    is_assert_panic(call_result.result.as_ref())
        || matches!(call_result.exit_reason, Some(InstructionResult::InvalidFEOpcode))
        || is_revert_assertion_failure(call_result.result.as_ref())
        || is_cheatcode_assert_revert(call_result)
}

fn is_assert_panic(data: &[u8]) -> bool {
    Panic::abi_decode(data).is_ok_and(|panic| panic == PanicKind::Assert.into())
}

fn is_revert_assertion_failure(data: &[u8]) -> bool {
    Revert::abi_decode(data).is_ok_and(|revert| revert.reason.contains(ASSERTION_FAILED_PREFIX))
}

fn is_cheatcode_assert_revert<FEN: FoundryEvmNetwork>(call_result: &RawCallResult<FEN>) -> bool {
    fn decoded_cheatcode_message(data: &[u8]) -> Option<String> {
        Vm::VmErrors::abi_decode(data).ok().map(|error| error.to_string())
    }

    call_result.reverter == Some(CHEATCODE_ADDRESS)
        && decoded_cheatcode_message(call_result.result.as_ref())
            .is_some_and(|message| message.starts_with(ASSERTION_FAILED_PREFIX))
}

fn logged_assertion_failure<FEN: FoundryEvmNetwork>(call_result: &RawCallResult<FEN>) -> bool {
    call_result
        .logs
        .iter()
        .filter_map(decode_console_log)
        .any(|msg| msg.starts_with(ASSERTION_FAILED_PREFIX))
}

/// Returns whether the current fuzz call should be treated as an assertion failure.
///
/// This covers Solidity `assert`, legacy invalid-opcode assertions, `vm.assert*` reverts, and the
/// non-reverting `GLOBAL_FAIL_SLOT` path used when `assertions_revert = false`.
pub(crate) fn did_fail_on_assert<FEN: FoundryEvmNetwork>(
    call_result: &RawCallResult<FEN>,
    state_changeset: &StateChangeset,
) -> bool {
    is_assertion_failure(call_result)
        || call_result.has_state_snapshot_failure
        || Executor::<FEN>::has_pending_global_failure(state_changeset)
        || logged_assertion_failure(call_result)
}

/// Given the executor state, asserts that no invariant has been broken. Otherwise, it fills the
/// external `invariant_failures.failed_invariant` map and returns a generic error.
/// Either returns the call result if successful, or nothing if there was an error.
pub(crate) fn assert_invariants<FEN: FoundryEvmNetwork>(
    invariant_contract: &InvariantContract<'_>,
    invariant_config: &InvariantConfig,
    targeted_contracts: &FuzzRunIdentifiedContracts,
    executor: &Executor<FEN>,
    calldata: &[BasicTxDetails],
    invariant_failures: &mut InvariantFailures,
) -> Result<()> {
    let inner_sequence = invariant_inner_sequence(executor);

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
                    call_result,
                    &inner_sequence,
                )),
            );
        }
    }

    Ok(())
}

/// Helper function to initialize invariant inner sequence.
fn invariant_inner_sequence<FEN: FoundryEvmNetwork>(
    executor: &Executor<FEN>,
) -> Vec<Option<BasicTxDetails>> {
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
///
/// For optimization mode (int256 return), tracks the max value but never fails on invariant.
/// For check mode, asserts the invariant and fails if broken.
///
/// `handler_target` / `handler_selector` identify the just-executed handler call; they are
/// used to attribute handler-side assertion failures so they can be tracked independently
/// from invariant predicate violations.
#[allow(clippy::too_many_arguments)]
pub(crate) fn can_continue<FEN: FoundryEvmNetwork>(
    invariant_contract: &InvariantContract<'_>,
    invariant_test: &mut InvariantTest,
    invariant_run: &mut InvariantTestRun<FEN>,
    invariant_config: &InvariantConfig,
    call_result: RawCallResult<FEN>,
    state_changeset: &StateChangeset,
    handler_target: Address,
    handler_selector: Selector,
    pre_merge_edges_hash: Option<B256>,
) -> Result<bool> {
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
                invariant_contract.primary_invariant_fn.abi_encode_input(&[])?.into(),
            )?;
            if success
                && inv_result.result.len() >= 32
                && let Some(value) = I256::try_from_be_slice(&inv_result.result[..32])
            {
                invariant_test.update_optimization_value(value, &invariant_run.inputs);
                // Track the best value and its prefix length for this run
                // (used for corpus persistence — materialized once at run end).
                if invariant_run.optimization_value.is_none_or(|prev| value > prev) {
                    invariant_run.optimization_value = Some(value);
                    invariant_run.optimization_prefix_len = invariant_run.inputs.len();
                }
            }
        } else {
            // Check mode: assert invariants and fail if broken.
            assert_invariants(
                invariant_contract,
                invariant_config,
                &invariant_test.targeted_contracts,
                &invariant_run.executor,
                &invariant_run.inputs,
                &mut invariant_test.test_data.failures,
            )?;
        }
    } else {
        let is_assert_failure = did_fail_on_assert(&call_result, state_changeset);
        let reverted = call_result.reverted;

        if reverted {
            invariant_test.test_data.failures.reverts += 1;
        }

        if is_assert_failure {
            // Handler-side assertion: a unique bug attributable to the *handler call*, not to
            // any of the live `invariant_*` predicates. Dedup by the `(reverter, selector)`
            // site so the same handler function asserting via different code paths counts as
            // a single bug (Echidna/Medusa semantics). On collision the shortest
            // `call_sequence` wins, so persisted reproducers stay minimal.
            let target = handler_target;
            let selector = handler_selector;
            let fingerprint = handler_edge_fingerprint(pre_merge_edges_hash, target, selector);

            // Skip building case data if we already have a strictly shorter repro for this
            // site — common when a handler asserts repeatedly.
            let already_minimal = invariant_test
                .test_data
                .failures
                .broken_handlers
                .get(&(target, selector))
                .is_some_and(|f| f.call_sequence.len() <= invariant_run.inputs.len());

            if !already_minimal {
                let case_data = FailedInvariantCaseData::new(
                    invariant_contract,
                    invariant_config.shrink_run_limit,
                    invariant_config.fail_on_revert,
                    &invariant_test.targeted_contracts,
                    &invariant_run.inputs,
                    call_result,
                    &[],
                )
                .with_assertion_failure(true);
                let revert_reason = case_data.revert_reason;
                invariant_test.test_data.failures.revert_reason = Some(revert_reason.clone());
                let call_sequence = invariant_run.inputs.clone();
                let original_sequence_len = call_sequence.len();
                invariant_test.test_data.failures.record_handler_failure(HandlerAssertionFailure {
                    reverter: target,
                    selector,
                    call_sequence,
                    original_sequence_len,
                    revert_reason,
                    assertion_failure: true,
                    edge_fingerprint: fingerprint,
                });
            }

            if reverted && !is_optimization && !invariant_config.has_delay() {
                // Mirror the standard reverted-input pop so the input doesn't appear in
                // subsequent prefixes. Delay-enabled campaigns keep reverted calls so
                // shrinking can preserve their warp/roll contribution.
                invariant_run.inputs.pop();
            }

            return Ok(invariant_test
                .test_data
                .failures
                .can_continue(invariant_contract.invariant_fns.len()));
        }

        // Non-assertion revert: per-invariant `fail_on_revert` semantics still mark the
        // affected invariants as broken (assertion failures are now routed above and never
        // flow into this filter).
        let failing_invariants: Vec<_> = invariant_contract
            .invariant_fns
            .iter()
            .filter(|(invariant, fail_on_revert)| {
                *fail_on_revert && !invariant_test.test_data.failures.has_failure(invariant)
            })
            .collect();

        if !failing_invariants.is_empty() {
            let base = FailedInvariantCaseData::new(
                invariant_contract,
                invariant_config.shrink_run_limit,
                invariant_config.fail_on_revert,
                &invariant_test.targeted_contracts,
                &invariant_run.inputs,
                call_result,
                &[],
            )
            .with_assertion_failure(false);
            invariant_test.test_data.failures.revert_reason = Some(base.revert_reason.clone());

            for (invariant, fail_on_revert) in failing_invariants {
                let mut data = base.clone();
                data.fail_on_revert = *fail_on_revert;
                invariant_test
                    .test_data
                    .failures
                    .record_failure(invariant, InvariantFuzzError::Revert(data));
            }
        }

        if reverted && !is_optimization && !invariant_config.has_delay() {
            // If we don't fail test on revert then remove the reverted call from inputs.
            // Delay-enabled campaigns keep reverted calls so shrinking can preserve their
            // warp/roll contribution when building the final counterexample.
            invariant_run.inputs.pop();
        }
    }

    Ok(invariant_test.test_data.failures.can_continue(invariant_contract.invariant_fns.len()))
}

/// Given the executor state, asserts conditions within `afterInvariant` function.
/// If call fails then the invariant test is considered failed.
pub(crate) fn assert_after_invariant<FEN: FoundryEvmNetwork>(
    invariant_contract: &InvariantContract<'_>,
    invariant_test: &mut InvariantTest,
    invariant_run: &InvariantTestRun<FEN>,
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
            call_result,
            &[],
        );
        invariant_test.set_error(
            invariant_contract.primary_invariant_fn,
            InvariantFuzzError::BrokenInvariant(case_data),
        );
    }
    Ok(success)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Bytes;
    use foundry_evm_core::evm::EthEvmNetwork;

    fn panic_payload(code: u8) -> Bytes {
        let mut payload = vec![0_u8; 36];
        payload[..4].copy_from_slice(&[0x4e, 0x48, 0x7b, 0x71]);
        payload[35] = code;
        payload.into()
    }

    #[test]
    fn detects_assert_panic_code() {
        let call_result = RawCallResult::<EthEvmNetwork> {
            reverted: true,
            result: panic_payload(0x01),
            ..Default::default()
        };
        assert!(is_assertion_failure(&call_result));
    }

    #[test]
    fn ignores_non_assert_panic_code() {
        let call_result = RawCallResult::<EthEvmNetwork> {
            reverted: true,
            result: panic_payload(0x11),
            ..Default::default()
        };
        assert!(!is_assertion_failure(&call_result));
    }

    #[test]
    fn detects_legacy_invalid_opcode_assert() {
        let call_result = RawCallResult::<EthEvmNetwork> {
            reverted: true,
            exit_reason: Some(InstructionResult::InvalidFEOpcode),
            ..Default::default()
        };
        assert!(is_assertion_failure(&call_result));
    }

    #[test]
    fn detects_vm_assert_revert() {
        let call_result = RawCallResult::<EthEvmNetwork> {
            reverted: true,
            result: Vm::CheatcodeError { message: format!("{ASSERTION_FAILED_PREFIX}: 1 != 2") }
                .abi_encode()
                .into(),
            reverter: Some(CHEATCODE_ADDRESS),
            ..Default::default()
        };
        assert!(is_assertion_failure(&call_result));
    }

    #[test]
    fn detects_assertion_failure_revert_reason() {
        let call_result = RawCallResult::<EthEvmNetwork> {
            reverted: true,
            result: Revert { reason: format!("{ASSERTION_FAILED_PREFIX}: expected") }
                .abi_encode()
                .into(),
            ..Default::default()
        };
        assert!(is_assertion_failure(&call_result));
    }

    #[test]
    fn ignores_empty_cheatcode_revert() {
        let call_result = RawCallResult::<EthEvmNetwork> {
            reverted: true,
            result: Bytes::new(),
            reverter: Some(CHEATCODE_ADDRESS),
            ..Default::default()
        };
        assert!(!is_assertion_failure(&call_result));
    }
}
