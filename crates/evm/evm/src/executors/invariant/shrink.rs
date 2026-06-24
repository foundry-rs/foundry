use crate::executors::{
    EarlyExit, EvmError, Executor, RawCallResult,
    invariant::{
        call_after_invariant_function, call_invariant_function,
        error::{handler_edge_fingerprint, snapshot_edge_fingerprint},
        execute_tx,
        result::did_fail_on_assert,
    },
};
use alloy_json_abi::Function;
use alloy_primitives::{Address, B256, Bytes, I256, Selector, U256};
use foundry_config::InvariantConfig;
use foundry_evm_core::{
    FoundryBlock, constants::MAGIC_ASSUME, decode::RevertDecoder, evm::FoundryEvmNetwork,
};
use foundry_evm_fuzz::{BasicTxDetails, invariant::InvariantContract};
use indicatif::ProgressBar;
use proptest::bits::{BitSetLike, VarBitSet};
use revm::context::Block;

/// Shrinker for a call sequence failure.
/// Iterates sequence call sequence top down and removes calls one by one.
/// If the failure is still reproducible with removed call then moves to the next one.
/// If the failure is not reproducible then restore removed call and moves to next one.
#[derive(Debug)]
struct CallSequenceShrinker {
    /// Length of call sequence to be shrunk.
    call_sequence_len: usize,
    /// Call ids contained in current shrunk sequence.
    included_calls: VarBitSet,
}

impl CallSequenceShrinker {
    fn new(call_sequence_len: usize) -> Self {
        Self { call_sequence_len, included_calls: VarBitSet::saturated(call_sequence_len) }
    }

    /// Return candidate shrink sequence to be tested, by removing ids from original sequence.
    fn current(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.call_sequence_len).filter(|&call_id| self.included_calls.test(call_id))
    }

    /// Advance to the next call index, wrapping around to 0 at the end.
    const fn next_index(&self, call_idx: usize) -> usize {
        if call_idx + 1 == self.call_sequence_len { 0 } else { call_idx + 1 }
    }
}

/// How `run_shrink_loop` handles a predicate error.
#[derive(Clone, Copy)]
enum ShrinkErrorPolicy {
    /// "Bug still present" — keep the call removed (legacy `shrink_sequence` behavior).
    KeepRemoved,
    /// "Bug gone" — restore the call. Used by handler shrink so a replay error never
    /// produces a sequence that no longer reproduces the anchor.
    RestoreRemoved,
}

/// Per-call decision returned by callbacks driving `replay_sequence`. `Continue` hands
/// the result back so non-reverted calls auto-commit; `Stop` short-circuits.
#[expect(clippy::large_enum_variant)]
enum ReplayDecision<T, FEN: FoundryEvmNetwork> {
    Stop(T),
    Continue(RawCallResult<FEN>),
}

/// Options controlling how `check_sequence` evaluates a candidate call sequence.
pub struct CheckSequenceOptions<'a> {
    pub accumulate_warp_roll: bool,
    pub fail_on_revert: bool,
    pub expect_assertion_failure: bool,
    pub call_after_invariant: bool,
    pub rd: Option<&'a RevertDecoder>,
}

pub type CheckSequenceOutcome = (bool, bool, Option<String>, usize, usize);

pub struct ShrunkSequence {
    pub calls: Vec<BasicTxDetails>,
    pub result: Option<CheckSequenceOutcome>,
}

/// Result of a strict handler-bug replay: anchor asserts, no earlier call asserts, and the
/// recomputed edge fingerprint identifies which path the assertion took.
#[derive(Debug)]
pub struct HandlerReplayOutcome {
    pub anchor_asserted: bool,
    pub revert_reason: Option<String>,
    /// Normalized via `handler_edge_fingerprint` so callers can compare directly.
    pub anchor_fingerprint: B256,
}

/// Resets the progress bar before each shrink. `position = Some((i, N))` renders
/// `[i/N] Shrink: <label>` for multi-invariant campaigns.
pub(crate) fn reset_shrink_progress(
    config: &InvariantConfig,
    progress: Option<&ProgressBar>,
    label: &str,
    position: Option<(usize, usize)>,
) {
    if let Some(progress) = progress {
        progress.set_length(config.shrink_run_limit as u64);
        progress.reset();
        let message = match position {
            Some((current, total)) if total > 1 => {
                format!(" [{current}/{total}] Shrink: {label}")
            }
            _ => format!(" Shrink: {label}"),
        };
        progress.set_message(message);
    }
}

/// Applies accumulated warp/roll to a call, returning a modified copy.
fn apply_warp_roll(call: &BasicTxDetails, warp: U256, roll: U256) -> BasicTxDetails {
    let mut result = call.clone();
    if warp > U256::ZERO {
        result.warp = Some(warp);
    }
    if roll > U256::ZERO {
        result.roll = Some(roll);
    }
    result
}

/// Applies warp/roll adjustments directly to the executor's environment.
fn apply_warp_roll_to_env<FEN: FoundryEvmNetwork>(
    executor: &mut Executor<FEN>,
    warp: U256,
    roll: U256,
) {
    if warp > U256::ZERO || roll > U256::ZERO {
        let ts = executor.evm_env().block_env.timestamp();
        let num = executor.evm_env().block_env.number();
        executor.evm_env_mut().block_env.set_timestamp(ts + warp);
        executor.evm_env_mut().block_env.set_number(num + roll);

        let block_env = executor.evm_env().block_env.clone();
        if let Some(cheatcodes) = executor.inspector_mut().cheatcodes.as_mut() {
            if let Some(block) = cheatcodes.block.as_mut() {
                let bts = block.timestamp();
                let bnum = block.number();
                block.set_timestamp(bts + warp);
                block.set_number(bnum + roll);
            } else {
                cheatcodes.block = Some(block_env);
            }
        }
    }
}

/// Builds the final shrunk sequence from the shrinker state.
///
/// When `accumulate_warp_roll` is enabled, warp/roll from removed calls is folded into the next
/// kept call so the final sequence remains reproducible.
fn build_shrunk_sequence(
    calls: &[BasicTxDetails],
    shrinker: &CallSequenceShrinker,
    accumulate_warp_roll: bool,
) -> Vec<BasicTxDetails> {
    if !accumulate_warp_roll {
        return shrinker.current().map(|idx| calls[idx].clone()).collect();
    }

    let mut result = Vec::new();
    let mut accumulated_warp = U256::ZERO;
    let mut accumulated_roll = U256::ZERO;

    for (idx, call) in calls.iter().enumerate() {
        accumulated_warp += call.warp.unwrap_or(U256::ZERO);
        accumulated_roll += call.roll.unwrap_or(U256::ZERO);

        if shrinker.included_calls.test(idx) {
            result.push(apply_warp_roll(call, accumulated_warp, accumulated_roll));
            accumulated_warp = U256::ZERO;
            accumulated_roll = U256::ZERO;
        }
    }

    result
}

/// Shared shrink loop driver. Tries to drop each call; `predicate` returns whether the
/// candidate still triggers the bug.
fn run_shrink_loop<P>(
    config: &InvariantConfig,
    calls_len: usize,
    progress: Option<&ProgressBar>,
    early_exit: &EarlyExit,
    error_policy: ShrinkErrorPolicy,
    mut predicate: P,
) -> CallSequenceShrinker
where
    P: FnMut(&CallSequenceShrinker) -> eyre::Result<bool>,
{
    let mut shrinker = CallSequenceShrinker::new(calls_len);
    let mut call_idx = 0;

    for _ in 0..config.shrink_run_limit {
        if early_exit.should_stop() {
            break;
        }

        // Already-removed indices have nothing to drop.
        if !shrinker.included_calls.test(call_idx) {
            call_idx = shrinker.next_index(call_idx);
            continue;
        }

        shrinker.included_calls.clear(call_idx);

        let bug_still_present = match predicate(&shrinker) {
            Ok(b) => b,
            Err(_) => matches!(error_policy, ShrinkErrorPolicy::KeepRemoved),
        };
        if bug_still_present {
            if shrinker.included_calls.count() == 1 {
                break;
            }
        } else {
            shrinker.included_calls.set(call_idx);
        }

        if let Some(progress) = progress {
            progress.inc(1);
        }
        call_idx = shrinker.next_index(call_idx);
    }

    shrinker
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn shrink_sequence<FEN: FoundryEvmNetwork>(
    config: &InvariantConfig,
    invariant_contract: &InvariantContract<'_>,
    target_invariant: &Function,
    calls: &[BasicTxDetails],
    expect_assertion_failure: bool,
    executor: &Executor<FEN>,
    rd: Option<&RevertDecoder>,
    progress: Option<&ProgressBar>,
    early_exit: &EarlyExit,
) -> eyre::Result<ShrunkSequence> {
    trace!(target: "forge::test", "Shrinking sequence of {} calls.", calls.len());

    let target_address = invariant_contract.address;
    let calldata: Bytes = target_invariant.selector().to_vec().into();
    // Special case test: the invariant is *unsatisfiable* - it took 0 calls to
    // break the invariant -- consider emitting a warning.
    let (_, success) = call_invariant_function(executor, target_address, calldata.clone())?;
    if !success {
        return Ok(ShrunkSequence { calls: vec![], result: None });
    }

    let accumulate_warp_roll = config.has_delay();
    let mut last_result = None;
    let mut last_result_matches_shrinker = true;
    let shrinker = run_shrink_loop(
        config,
        calls.len(),
        progress,
        early_exit,
        // Preserve legacy invariant-shrink behavior: errors during candidate evaluation
        // do not roll back the removal.
        ShrinkErrorPolicy::KeepRemoved,
        |shrinker| {
            let result = match check_sequence(
                executor.clone(),
                calls,
                shrinker.current().collect(),
                target_address,
                calldata.clone(),
                CheckSequenceOptions {
                    accumulate_warp_roll,
                    fail_on_revert: config.fail_on_revert,
                    expect_assertion_failure,
                    call_after_invariant: invariant_contract.call_after_invariant,
                    rd,
                },
            ) {
                Ok(result) => result,
                Err(err) => {
                    last_result_matches_shrinker = false;
                    return Err(err);
                }
            };
            // Bug still present iff the invariant predicate did not pass.
            let success = result.0;
            if !success {
                last_result = Some(result);
                last_result_matches_shrinker = true;
            }
            Ok(!success)
        },
    );

    let shrunk = build_shrunk_sequence(calls, &shrinker, accumulate_warp_roll);
    let result = if last_result_matches_shrinker {
        last_result
    } else {
        match check_sequence(
            executor.clone(),
            &shrunk,
            (0..shrunk.len()).collect(),
            target_address,
            calldata,
            CheckSequenceOptions {
                accumulate_warp_roll: false,
                fail_on_revert: config.fail_on_revert,
                expect_assertion_failure,
                call_after_invariant: invariant_contract.call_after_invariant,
                rd,
            },
        ) {
            Ok(result) => Some(result),
            Err(err) => {
                trace!(target: "forge::test", %err, "failed to recompute shrunk replay metrics");
                None
            }
        }
    };

    Ok(ShrunkSequence { calls: shrunk, result })
}

/// Replays `sequence` (indices into `calls`) against `executor`. When
/// `accumulate_warp_roll` is set, warp/roll from skipped calls is folded into the next
/// included call. `on_call` may stop early; otherwise non-reverted calls are committed.
fn replay_sequence<FEN, T, F>(
    executor: &mut Executor<FEN>,
    calls: &[BasicTxDetails],
    sequence: &[usize],
    accumulate_warp_roll: bool,
    mut on_call: F,
) -> eyre::Result<Option<T>>
where
    FEN: FoundryEvmNetwork,
    F: FnMut(usize, RawCallResult<FEN>) -> eyre::Result<ReplayDecision<T, FEN>>,
{
    // Fast path: no warp/roll accumulation → iterate only kept indices (O(k)) and pass
    // `&calls[idx]` directly to skip the per-call `BasicTxDetails` clone.
    if !accumulate_warp_roll {
        for &idx in sequence {
            let call_result = execute_tx(executor, &calls[idx])?;
            match on_call(idx, call_result)? {
                ReplayDecision::Stop(val) => return Ok(Some(val)),
                ReplayDecision::Continue(mut call_result) => {
                    if !call_result.reverted {
                        executor.commit(&mut call_result);
                    }
                }
            }
        }
        return Ok(None);
    }

    // Accumulating path: must scan the full `calls` so warp/roll from skipped txs lands on
    // the next kept tx as a concrete delta.
    let mut accumulated_warp = U256::ZERO;
    let mut accumulated_roll = U256::ZERO;
    let mut seq_iter = sequence.iter().peekable();

    for (idx, tx) in calls.iter().enumerate() {
        accumulated_warp += tx.warp.unwrap_or(U256::ZERO);
        accumulated_roll += tx.roll.unwrap_or(U256::ZERO);
        if seq_iter.peek() != Some(&&idx) {
            continue;
        }
        seq_iter.next();

        let executed = apply_warp_roll(tx, accumulated_warp, accumulated_roll);
        let call_result = execute_tx(executor, &executed)?;

        match on_call(idx, call_result)? {
            ReplayDecision::Stop(val) => return Ok(Some(val)),
            ReplayDecision::Continue(mut call_result) => {
                if !call_result.reverted {
                    executor.commit(&mut call_result);
                }
            }
        }

        accumulated_warp = U256::ZERO;
        accumulated_roll = U256::ZERO;
    }

    Ok(None)
}

/// Checks if the given call sequence breaks the invariant.
///
/// Used in shrinking phase for checking candidate sequences and in replay failures phase to test
/// persisted failures.
/// Returns the result of invariant check (and afterInvariant call if needed) and if sequence was
/// entirely applied.
///
/// When `options.accumulate_warp_roll` is enabled, warp/roll from removed calls is folded into the
/// next kept call so the candidate sequence stays representable as a concrete counterexample.
pub fn check_sequence<FEN: FoundryEvmNetwork>(
    mut executor: Executor<FEN>,
    calls: &[BasicTxDetails],
    sequence: Vec<usize>,
    test_address: Address,
    calldata: Bytes,
    options: CheckSequenceOptions<'_>,
) -> eyre::Result<(bool, bool, Option<String>, usize, usize)> {
    let mut calls_executed = 0;
    let mut reverts = 0;
    let early = replay_sequence(
        &mut executor,
        calls,
        &sequence,
        options.accumulate_warp_roll,
        |_idx, call_result| {
            calls_executed += 1;
            // Ignore calls reverted with `MAGIC_ASSUME`. This is needed to handle failed
            // scenarios that are replayed with a modified version of test driver (that use
            // new `vm.assume` cheatcodes).
            if call_result.result.as_ref() == MAGIC_ASSUME {
                return Ok(ReplayDecision::Continue(call_result));
            }
            if call_result.reverted {
                reverts += 1;
            }
            if did_fail_on_assert(&call_result, &call_result.state_changeset) {
                return Ok(ReplayDecision::Stop((
                    false,
                    false,
                    assertion_failure_reason(call_result, options.rd),
                    calls_executed,
                    reverts,
                )));
            }
            if call_result.reverted && options.fail_on_revert {
                if options.expect_assertion_failure {
                    return Ok(ReplayDecision::Stop((true, false, None, calls_executed, reverts)));
                }
                return Ok(ReplayDecision::Stop((
                    false,
                    false,
                    call_failure_reason(call_result, options.rd),
                    calls_executed,
                    reverts,
                )));
            }
            Ok(ReplayDecision::Continue(call_result))
        },
    )?;
    if let Some(result) = early {
        return Ok(result);
    }

    // Unlike optimization mode we intentionally do not apply trailing warp/roll before the
    // invariant call: those delays would not be representable in the final shrunk sequence.
    let (success, replayed_entirely, reason) =
        finish_sequence_check(&executor, test_address, calldata, &options)?;
    Ok((success, replayed_entirely, reason, calls_executed, reverts))
}

fn finish_sequence_check<FEN: FoundryEvmNetwork>(
    executor: &Executor<FEN>,
    test_address: Address,
    calldata: Bytes,
    options: &CheckSequenceOptions<'_>,
) -> eyre::Result<(bool, bool, Option<String>)> {
    let handle_terminal_failure = |call_result: RawCallResult<FEN>| {
        let should_ignore_failure = options.expect_assertion_failure
            && !executor.has_global_failure(&call_result.state_changeset)
            && !did_fail_on_assert(&call_result, &call_result.state_changeset);

        if should_ignore_failure {
            return (true, true, None);
        }

        let reason = if options.expect_assertion_failure {
            assertion_failure_reason(call_result, options.rd)
        } else {
            call_failure_reason(call_result, options.rd)
        };

        (false, true, reason)
    };

    let (invariant_result, mut success) =
        call_invariant_function(executor, test_address, calldata)?;
    if !success {
        return Ok(handle_terminal_failure(invariant_result));
    }

    // Check after invariant result if invariant is success and `afterInvariant` function is
    // declared.
    if success && options.call_after_invariant {
        let (after_invariant_result, after_invariant_success) =
            call_after_invariant_function(executor, test_address)?;
        success = after_invariant_success;
        if !success {
            return Ok(handle_terminal_failure(after_invariant_result));
        }
    }

    Ok((success, true, None))
}

fn call_failure_reason<FEN: FoundryEvmNetwork>(
    call_result: RawCallResult<FEN>,
    rd: Option<&RevertDecoder>,
) -> Option<String> {
    match call_result.into_evm_error(rd) {
        EvmError::Execution(err) => Some(err.reason),
        _ => None,
    }
}

fn assertion_failure_reason<FEN: FoundryEvmNetwork>(
    call_result: RawCallResult<FEN>,
    rd: Option<&RevertDecoder>,
) -> Option<String> {
    call_failure_reason(call_result, rd).or_else(|| Some("assertion failed".to_string()))
}

/// Shrinks a call sequence to the shortest sequence that still produces the target optimization
/// value. This is specifically for optimization mode where we want to find the minimal sequence
/// that achieves the maximum value.
///
/// Unlike `shrink_sequence` (for check mode), this function:
/// - Accumulates warp/roll values from removed calls into the next kept call
/// - Checks for target value equality rather than invariant failure
#[expect(clippy::too_many_arguments)]
pub(crate) fn shrink_sequence_value<FEN: FoundryEvmNetwork>(
    config: &InvariantConfig,
    invariant_contract: &InvariantContract<'_>,
    target_invariant: &Function,
    calls: &[BasicTxDetails],
    executor: &Executor<FEN>,
    target_value: I256,
    progress: Option<&ProgressBar>,
    early_exit: &EarlyExit,
) -> eyre::Result<Vec<BasicTxDetails>> {
    trace!(target: "forge::test", "Shrinking optimization sequence of {} calls for target value {}.", calls.len(), target_value);

    let target_address = invariant_contract.address;
    let calldata: Bytes = target_invariant.selector().to_vec().into();

    // Special case: check if target value is achieved with 0 calls.
    if check_sequence_value(executor.clone(), calls, vec![], target_address, calldata.clone())?
        == Some(target_value)
    {
        return Ok(vec![]);
    }

    let mut call_idx = 0;
    let mut shrinker = CallSequenceShrinker::new(calls.len());

    for _ in 0..config.shrink_run_limit {
        if early_exit.should_stop() {
            break;
        }

        shrinker.included_calls.clear(call_idx);

        let keeps_target = check_sequence_value(
            executor.clone(),
            calls,
            shrinker.current().collect(),
            target_address,
            calldata.clone(),
        )? == Some(target_value);

        if keeps_target {
            if shrinker.included_calls.count() == 1 {
                break;
            }
        } else {
            shrinker.included_calls.set(call_idx);
        }

        if let Some(progress) = progress {
            progress.inc(1);
        }

        call_idx = shrinker.next_index(call_idx);
    }

    Ok(build_shrunk_sequence(calls, &shrinker, true))
}

/// Replays a handler-bug sequence and returns whether the anchor still asserts on the same
/// path. Rejects sequences with a pre-anchor assertion (would be a different bug).
pub fn replay_handler_failure_sequence<FEN: FoundryEvmNetwork>(
    mut executor: Executor<FEN>,
    calls: &[BasicTxDetails],
    sequence: Vec<usize>,
    accumulate_warp_roll: bool,
    rd: Option<&RevertDecoder>,
) -> eyre::Result<HandlerReplayOutcome> {
    let Some(&anchor_idx) = sequence.last() else {
        return Ok(HandlerReplayOutcome {
            anchor_asserted: false,
            revert_reason: None,
            anchor_fingerprint: B256::ZERO,
        });
    };

    let outcome = replay_sequence(
        &mut executor,
        calls,
        &sequence,
        accumulate_warp_roll,
        |idx, call_result| {
            let asserted = did_fail_on_assert(&call_result, &call_result.state_changeset);
            if idx == anchor_idx {
                let snapshot = snapshot_edge_fingerprint(&call_result);
                let anchor = &calls[anchor_idx];
                let reverter = anchor.call_details.target;
                let selector_bytes: [u8; 4] = anchor
                    .call_details
                    .calldata
                    .get(..4)
                    .and_then(|s| s.try_into().ok())
                    .unwrap_or_default();
                let selector = Selector::from(selector_bytes);
                let fingerprint = handler_edge_fingerprint(snapshot, reverter, selector);
                let reason =
                    if asserted { assertion_failure_reason(call_result, rd) } else { None };
                return Ok(ReplayDecision::Stop(HandlerReplayOutcome {
                    anchor_asserted: asserted,
                    revert_reason: reason,
                    anchor_fingerprint: fingerprint,
                }));
            }
            if asserted {
                // Pre-anchor assertion = different bug; reject.
                return Ok(ReplayDecision::Stop(HandlerReplayOutcome {
                    anchor_asserted: false,
                    revert_reason: None,
                    anchor_fingerprint: B256::ZERO,
                }));
            }
            Ok(ReplayDecision::Continue(call_result))
        },
    )?;

    Ok(outcome.unwrap_or(HandlerReplayOutcome {
        anchor_asserted: false,
        revert_reason: None,
        anchor_fingerprint: B256::ZERO,
    }))
}

/// Shrinks a handler-bug sequence to the shortest prefix that still asserts on the anchor
/// AND keeps the same edge fingerprint (so we don't change bug identity).
pub(crate) fn shrink_handler_sequence<FEN: FoundryEvmNetwork>(
    config: &InvariantConfig,
    calls: &[BasicTxDetails],
    expected_fingerprint: B256,
    executor: &Executor<FEN>,
    progress: Option<&ProgressBar>,
    early_exit: &EarlyExit,
) -> eyre::Result<Vec<BasicTxDetails>> {
    if calls.is_empty() {
        return Ok(vec![]);
    }
    let accumulate_warp_roll = config.has_delay();
    let shrinker = run_shrink_loop(
        config,
        calls.len(),
        progress,
        early_exit,
        ShrinkErrorPolicy::RestoreRemoved,
        |shrinker| {
            handler_sequence_still_triggers_bug(
                executor.clone(),
                calls,
                shrinker.current().collect(),
                accumulate_warp_roll,
                expected_fingerprint,
            )
        },
    );

    let shrunk = build_shrunk_sequence(calls, &shrinker, accumulate_warp_roll);

    // Verify shrunk repro; fall back to original on any failure.
    let verified = handler_sequence_still_triggers_bug(
        executor.clone(),
        calls,
        shrinker.current().collect(),
        accumulate_warp_roll,
        expected_fingerprint,
    )
    .unwrap_or(false);
    if verified { Ok(shrunk) } else { Ok(calls.to_vec()) }
}

/// Shrink predicate: anchor asserts on the same path as the originally recorded bug.
fn handler_sequence_still_triggers_bug<FEN: FoundryEvmNetwork>(
    executor: Executor<FEN>,
    calls: &[BasicTxDetails],
    sequence: Vec<usize>,
    accumulate_warp_roll: bool,
    expected_fingerprint: B256,
) -> eyre::Result<bool> {
    let outcome =
        replay_handler_failure_sequence(executor, calls, sequence, accumulate_warp_roll, None)?;
    Ok(outcome.anchor_asserted && outcome.anchor_fingerprint == expected_fingerprint)
}

/// Executes a call sequence and returns the optimization value (int256) from the invariant
/// function. Used during shrinking for optimization mode.
///
/// Returns `None` if the invariant call fails or doesn't return a valid int256.
/// Unlike `check_sequence`, this applies warp/roll from ALL calls (including removed ones).
pub fn check_sequence_value<FEN: FoundryEvmNetwork>(
    mut executor: Executor<FEN>,
    calls: &[BasicTxDetails],
    sequence: Vec<usize>,
    test_address: Address,
    calldata: Bytes,
) -> eyre::Result<Option<I256>> {
    let mut accumulated_warp = U256::ZERO;
    let mut accumulated_roll = U256::ZERO;
    let mut seq_iter = sequence.iter().peekable();

    for (idx, tx) in calls.iter().enumerate() {
        accumulated_warp += tx.warp.unwrap_or(U256::ZERO);
        accumulated_roll += tx.roll.unwrap_or(U256::ZERO);

        if seq_iter.peek() == Some(&&idx) {
            seq_iter.next();

            let tx_with_accumulated = apply_warp_roll(tx, accumulated_warp, accumulated_roll);
            let mut call_result = execute_tx(&mut executor, &tx_with_accumulated)?;

            if !call_result.reverted {
                executor.commit(&mut call_result);
            }

            accumulated_warp = U256::ZERO;
            accumulated_roll = U256::ZERO;
        }
    }

    // Apply any remaining accumulated warp/roll before calling invariant.
    apply_warp_roll_to_env(&mut executor, accumulated_warp, accumulated_roll);

    let (inv_result, success) = call_invariant_function(&executor, test_address, calldata)?;

    if success
        && inv_result.result.len() >= 32
        && let Some(value) = I256::try_from_be_slice(&inv_result.result[..32])
    {
        return Ok(Some(value));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{CallSequenceShrinker, ShrinkErrorPolicy, build_shrunk_sequence, run_shrink_loop};
    use crate::executors::EarlyExit;
    use alloy_primitives::{Address, Bytes, U256};
    use foundry_config::InvariantConfig;
    use foundry_evm_fuzz::{BasicTxDetails, CallDetails};
    use proptest::bits::BitSetLike;

    fn tx(warp: Option<u64>, roll: Option<u64>) -> BasicTxDetails {
        BasicTxDetails {
            warp: warp.map(U256::from),
            roll: roll.map(U256::from),
            sender: Address::ZERO,
            call_details: CallDetails {
                target: Address::ZERO,
                calldata: Bytes::new(),
                value: None,
            },
        }
    }

    #[test]
    fn build_shrunk_sequence_accumulates_removed_delay_into_next_kept_call() {
        let calls = vec![tx(Some(3), Some(5)), tx(Some(7), Some(11)), tx(Some(13), Some(17))];
        let mut shrinker = CallSequenceShrinker::new(calls.len());
        shrinker.included_calls.clear(0);

        let shrunk = build_shrunk_sequence(&calls, &shrinker, true);

        assert_eq!(shrunk.len(), 2);
        assert_eq!(shrunk[0].warp, Some(U256::from(10)));
        assert_eq!(shrunk[0].roll, Some(U256::from(16)));
        assert_eq!(shrunk[1].warp, Some(U256::from(13)));
        assert_eq!(shrunk[1].roll, Some(U256::from(17)));
    }

    #[test]
    fn build_shrunk_sequence_does_not_move_trailing_delay_backward() {
        let calls = vec![tx(Some(3), Some(5)), tx(Some(7), Some(11))];
        let mut shrinker = CallSequenceShrinker::new(calls.len());
        shrinker.included_calls.clear(1);

        let shrunk = build_shrunk_sequence(&calls, &shrinker, true);

        assert_eq!(shrunk.len(), 1);
        assert_eq!(shrunk[0].warp, Some(U256::from(3)));
        assert_eq!(shrunk[0].roll, Some(U256::from(5)));
    }

    #[test]
    fn shrink_loop_keep_removed_treats_candidate_error_as_still_failing() {
        let config = InvariantConfig { shrink_run_limit: 1, ..Default::default() };
        let early_exit = EarlyExit::new(false);

        let shrinker =
            run_shrink_loop(&config, 2, None, &early_exit, ShrinkErrorPolicy::KeepRemoved, |_| {
                Err(eyre::eyre!("candidate replay failed"))
            });

        assert_eq!(shrinker.current().collect::<Vec<_>>(), vec![1]);
    }
}
