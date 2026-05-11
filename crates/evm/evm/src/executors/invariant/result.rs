use super::{
    InvariantFailures, InvariantFuzzError, InvariantMetrics, InvariantTest, InvariantTestRun,
    call_after_invariant_function, call_invariant_function, error::InvariantRunCtx,
};
use crate::executors::{Executor, RawCallResult};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::I256;
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
use proptest::test_runner::TestError;
use revm::interpreter::InstructionResult;
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
    )?;
    Ok(())
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
/// external `invariant_failures.failed_invariant` map.
///
/// Returns the first newly-broken invariant in declaration order (if any), so callers can
/// attribute the failure event without re-scanning `invariant_failures.errors` afterwards.
pub(crate) fn assert_invariants<'a, FEN: FoundryEvmNetwork>(
    invariant_contract: &InvariantContract<'a>,
    invariant_config: &InvariantConfig,
    targeted_contracts: &FuzzRunIdentifiedContracts,
    executor: &Executor<FEN>,
    calldata: &[BasicTxDetails],
    invariant_failures: &mut InvariantFailures,
) -> Result<Option<&'a Function>> {
    let inner_sequence = invariant_inner_sequence(executor);
    let mut first_broken: Option<&'a Function> = None;
    let ctx = InvariantRunCtx {
        contract: invariant_contract,
        config: invariant_config,
        targeted_contracts,
        calldata,
    };

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
            let case =
                ctx.failed_case(invariant, *fail_on_revert, false, call_result, &inner_sequence);
            invariant_failures.record_failure(invariant, InvariantFuzzError::BrokenInvariant(case));
            if first_broken.is_none() {
                first_broken = Some(*invariant);
            }
        }
    }

    Ok(first_broken)
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

/// Outcome of a per-call invariant check.
#[derive(Debug)]
pub(crate) struct ContinueOutcome<'a> {
    /// Whether the invariant campaign should keep running after this call.
    pub continues: bool,
    /// First newly-broken invariant produced by this call, in declaration order. Used by the
    /// executor to record the failure event without re-scanning the failures map.
    pub broken: Option<&'a Function>,
}

/// Returns if invariant test can continue and last successful call result of the invariant test
/// function (if it can continue).
///
/// For optimization mode (int256 return), tracks the max value but never fails on invariant.
/// For check mode, asserts the invariant and fails if broken.
pub(crate) fn can_continue<'a, FEN: FoundryEvmNetwork>(
    invariant_contract: &InvariantContract<'a>,
    invariant_test: &mut InvariantTest,
    invariant_run: &mut InvariantTestRun<FEN>,
    invariant_config: &InvariantConfig,
    call_result: RawCallResult<FEN>,
    state_changeset: &StateChangeset,
) -> Result<ContinueOutcome<'a>> {
    let is_optimization = invariant_contract.is_optimization();
    let mut broken: Option<&'a Function> = None;

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
                invariant_contract.anchor().abi_encode_input(&[])?.into(),
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
            broken = assert_invariants(
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

        // Collect which invariants should be marked as failed due to this revert/assertion.
        let failing_invariants: Vec<_> = invariant_contract
            .invariant_fns
            .iter()
            .filter(|(invariant, fail_on_revert)| {
                (is_assert_failure || *fail_on_revert)
                    && !invariant_test.test_data.failures.has_failure(invariant)
            })
            .collect();

        if let Some((first_invariant, _)) = failing_invariants.first() {
            broken = Some(*first_invariant);
            // Build a base case_data attributed to the first failing invariant; clone it for
            // each subsequent broken invariant, retagging name/selector/`fail_on_revert` so
            // every recorded failure points at its own invariant body.
            let base = InvariantRunCtx {
                contract: invariant_contract,
                config: invariant_config,
                targeted_contracts: &invariant_test.targeted_contracts,
                calldata: &invariant_run.inputs,
            }
            .failed_case(
                first_invariant,
                invariant_config.fail_on_revert,
                is_assert_failure,
                call_result,
                &[],
            );
            invariant_test.test_data.failures.revert_reason = Some(base.revert_reason.clone());

            for (invariant, fail_on_revert) in failing_invariants {
                let mut data = base.clone();
                data.fail_on_revert = *fail_on_revert;
                data.calldata = invariant.selector().to_vec().into();
                data.test_error = TestError::Fail(
                    format!("{}, reason: {}", invariant.name, data.revert_reason).into(),
                    invariant_run.inputs.clone(),
                );
                invariant_test.test_data.failures.record_failure(
                    invariant,
                    if is_assert_failure {
                        InvariantFuzzError::BrokenInvariant(data)
                    } else {
                        InvariantFuzzError::Revert(data)
                    },
                );
            }
        }

        if reverted && !is_optimization && !invariant_config.has_delay() {
            // If we don't fail test on revert then remove the reverted call from inputs.
            // Delay-enabled campaigns keep reverted calls so shrinking can preserve their
            // warp/roll contribution when building the final counterexample.
            invariant_run.inputs.pop();
        }
    }

    let continues =
        invariant_test.test_data.failures.can_continue(invariant_contract.invariant_fns.len());
    Ok(ContinueOutcome { continues, broken })
}

/// Given the executor state, asserts conditions within `afterInvariant` function.
///
/// Returns `Some(anchor)` if the hook failed (so the caller can record the failure event
/// without re-scanning the failures map), or `None` if the hook succeeded.
pub(crate) fn assert_after_invariant<'a, FEN: FoundryEvmNetwork>(
    invariant_contract: &InvariantContract<'a>,
    invariant_test: &mut InvariantTest,
    invariant_run: &InvariantTestRun<FEN>,
    invariant_config: &InvariantConfig,
) -> Result<Option<&'a Function>> {
    let (call_result, success) =
        call_after_invariant_function(&invariant_run.executor, invariant_contract.address)?;
    // Fail the test case if `afterInvariant` doesn't succeed.
    if success {
        return Ok(None);
    }
    // `afterInvariant` failures are contract-wide (no specific invariant body executed),
    // so attribute to the campaign anchor.
    let anchor = invariant_contract.anchor();
    let case_data = InvariantRunCtx {
        contract: invariant_contract,
        config: invariant_config,
        targeted_contracts: &invariant_test.targeted_contracts,
        calldata: &invariant_run.inputs,
    }
    .failed_case(anchor, invariant_config.fail_on_revert, false, call_result, &[]);
    invariant_test.set_error(anchor, InvariantFuzzError::BrokenInvariant(case_data));
    Ok(Some(anchor))
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
