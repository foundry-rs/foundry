use super::{
    InvariantFailures, InvariantFuzzError, InvariantMetrics, InvariantTest, InvariantTestRun,
    call_after_invariant_function, call_invariant_function, error::FailedInvariantCaseData,
};
use crate::executors::{Executor, RawCallResult};
use alloy_dyn_abi::JsonAbiExt;
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
use revm::interpreter::InstructionResult;
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
pub(crate) struct RichInvariantResults<FEN: FoundryEvmNetwork> {
    pub(crate) can_continue: bool,
    pub(crate) call_result: Option<RawCallResult<FEN>>,
}

impl<FEN: FoundryEvmNetwork> RichInvariantResults<FEN> {
    pub(crate) const fn new(can_continue: bool, call_result: Option<RawCallResult<FEN>>) -> Self {
        Self { can_continue, call_result }
    }
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
) -> Result<Option<RawCallResult<FEN>>> {
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
pub(crate) fn can_continue<FEN: FoundryEvmNetwork>(
    invariant_contract: &InvariantContract<'_>,
    invariant_test: &mut InvariantTest<FEN>,
    invariant_run: &mut InvariantTestRun<FEN>,
    invariant_config: &InvariantConfig,
    call_result: RawCallResult<FEN>,
    state_changeset: &StateChangeset,
) -> Result<RichInvariantResults<FEN>> {
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
                // Track the best value and its prefix length for this run
                // (used for corpus persistence — materialized once at run end).
                if invariant_run.optimization_value.is_none_or(|prev| value > prev) {
                    invariant_run.optimization_value = Some(value);
                    invariant_run.optimization_prefix_len = invariant_run.inputs.len();
                }
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
        let invariant_data = &mut invariant_test.test_data;
        let is_assert_failure = did_fail_on_assert(&call_result, state_changeset);

        if call_result.reverted {
            invariant_data.failures.reverts += 1;
        }

        if is_assert_failure || (call_result.reverted && invariant_config.fail_on_revert) {
            let case_data = FailedInvariantCaseData::new(
                invariant_contract,
                invariant_config,
                &invariant_test.targeted_contracts,
                &invariant_run.inputs,
                call_result,
                &[],
            )
            .with_assertion_failure(is_assert_failure);
            invariant_data.failures.revert_reason = Some(case_data.revert_reason.clone());
            invariant_data.failures.error = Some(if is_assert_failure {
                InvariantFuzzError::BrokenInvariant(case_data)
            } else {
                InvariantFuzzError::Revert(case_data)
            });

            return Ok(RichInvariantResults::new(false, None));
        } else if call_result.reverted && !is_optimization && !invariant_config.has_delay() {
            // If we don't fail test on revert then remove the reverted call from inputs.
            // Delay-enabled campaigns keep reverted calls so shrinking can preserve their
            // warp/roll contribution when building the final counterexample.
            invariant_run.inputs.pop();
        }
    }
    Ok(RichInvariantResults::new(true, call_results))
}

/// Given the executor state, asserts conditions within `afterInvariant` function.
/// If call fails then the invariant test is considered failed.
pub(crate) fn assert_after_invariant<FEN: FoundryEvmNetwork>(
    invariant_contract: &InvariantContract<'_>,
    invariant_test: &mut InvariantTest<FEN>,
    invariant_run: &InvariantTestRun<FEN>,
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
