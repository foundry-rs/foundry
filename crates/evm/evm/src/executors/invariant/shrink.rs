use crate::executors::{
    EarlyExit, EvmError, Executor, RawCallResult,
    invariant::{
        IInvariantTest, call_after_invariant_function, call_invariant_function,
        error::{handler_edge_fingerprint, snapshot_edge_fingerprint},
        execute_tx,
        result::did_fail_on_assert,
    },
};
use alloy_json_abi::Function;
use alloy_primitives::{Address, B256, Bytes, I256, Selector, U256};
use alloy_sol_types::SolCall;
use foundry_common::ContractsByAddress;
use foundry_config::InvariantConfig;
use foundry_evm_core::{
    FoundryBlock, constants::MAGIC_ASSUME, decode::RevertDecoder, evm::FoundryEvmNetwork,
};
use foundry_evm_fuzz::{BaseCounterExample, BasicTxDetails, invariant::InvariantContract};
use indicatif::ProgressBar;
use proptest::bits::{BitSetLike, VarBitSet};
use revm::context::Block;
use std::{
    cell::Cell,
    collections::HashSet,
    fmt::Write,
    hash::Hash,
    time::{Duration, Instant},
};

const LIVE_SHRINK_SEQUENCE_EDGE_CALLS: usize = 16;
const LIVE_SHRINK_PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Shrinker for a call sequence failure.
/// Iterates sequence call sequence top down and removes calls one by one.
/// If the failure is still reproducible with removed call then moves to the next one.
/// If the failure is not reproducible then restore removed call and moves to next one.
#[derive(Debug)]
pub struct SequenceShrink {
    /// Length of call sequence to be shrunk.
    call_sequence_len: usize,
    /// Call ids contained in current shrunk sequence.
    included_calls: VarBitSet,
    /// Number of calls still included in the candidate sequence.
    included_count: usize,
}

impl SequenceShrink {
    pub fn new(call_sequence_len: usize) -> Self {
        Self {
            call_sequence_len,
            included_calls: VarBitSet::saturated(call_sequence_len),
            included_count: call_sequence_len,
        }
    }

    /// Return candidate shrink sequence to be tested, by removing ids from original sequence.
    pub fn current(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.call_sequence_len).filter(|&call_id| self.included_calls.test(call_id))
    }

    pub fn contains(&self, call_idx: usize) -> bool {
        self.included_calls.test(call_idx)
    }

    pub fn included_count(&self) -> usize {
        self.included_count
    }

    pub fn apply<T: Clone>(&self, calls: &[T]) -> Vec<T> {
        self.current().map(|idx| calls[idx].clone()).collect()
    }

    pub fn apply_with_accumulated_delay<T, D, A>(
        &self,
        calls: &[T],
        mut delay: D,
        mut apply_delay: A,
    ) -> Vec<T>
    where
        T: Clone,
        D: FnMut(&T) -> (Option<U256>, Option<U256>),
        A: FnMut(T, U256, U256) -> T,
    {
        let mut result = Vec::new();
        let mut accumulated_warp = U256::ZERO;
        let mut accumulated_roll = U256::ZERO;

        for (idx, call) in calls.iter().enumerate() {
            let (warp, roll) = delay(call);
            accumulated_warp += warp.unwrap_or(U256::ZERO);
            accumulated_roll += roll.unwrap_or(U256::ZERO);

            if self.contains(idx) {
                result.push(apply_delay(call.clone(), accumulated_warp, accumulated_roll));
                accumulated_warp = U256::ZERO;
                accumulated_roll = U256::ZERO;
            }
        }

        result
    }

    fn remove(&mut self, call_idx: usize) {
        if self.contains(call_idx) {
            self.included_calls.clear(call_idx);
            self.included_count -= 1;
        }
    }

    fn restore(&mut self, call_idx: usize) {
        if !self.contains(call_idx) {
            self.included_calls.set(call_idx);
            self.included_count += 1;
        }
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

/// Attempt counters collected while trying shrink candidates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShrinkRunStats {
    pub attempts: usize,
    pub accepted: usize,
}

/// Shared shrink attempt driver.
///
/// Candidate generation stays with each shrinker; this type only centralizes limit enforcement
/// and the "accept when the candidate still reproduces the bug" accounting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShrinkRun {
    max_attempts: usize,
    stats: ShrinkRunStats,
}

impl ShrinkRun {
    pub const fn new(max_attempts: usize) -> Self {
        Self { max_attempts, stats: ShrinkRunStats { attempts: 0, accepted: 0 } }
    }

    pub const fn can_try(&self) -> bool {
        self.stats.attempts < self.max_attempts
    }

    pub const fn remaining_attempts(&self) -> usize {
        self.max_attempts - self.stats.attempts
    }

    pub const fn finish(self) -> ShrinkRunStats {
        self.stats
    }

    pub fn try_candidate(&mut self, still_fails: impl FnOnce() -> bool) -> bool {
        self.try_candidate_decision(|| Some(still_fails())).unwrap_or(false)
    }

    fn try_candidate_decision(&mut self, decide: impl FnOnce() -> Option<bool>) -> Option<bool> {
        if !self.can_try() {
            return None;
        }

        let accepted = decide()?;
        self.stats.attempts += 1;
        if accepted {
            self.stats.accepted += 1;
        }
        Some(accepted)
    }
}

/// Shared key set for shrinkers that need to skip duplicate concrete replays.
#[derive(Clone, Debug)]
pub struct ShrinkCandidateKeys<K> {
    seen: HashSet<K>,
}

impl<K: Eq + Hash> ShrinkCandidateKeys<K> {
    pub fn new(initial: K) -> Self {
        Self { seen: HashSet::from([initial]) }
    }

    pub fn insert(&mut self, key: K) -> bool {
        self.seen.insert(key)
    }
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

/// Concrete failure site observed while replaying a sequence through [`check_sequence`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckSequenceFailureSite {
    SequenceCall { target: Address, selector: Selector, fingerprint: B256 },
    Invariant { target: Address, selector: Selector, fingerprint: B256 },
    AfterInvariant { target: Address, selector: Selector, fingerprint: B256 },
}

/// Outcome from replaying an invariant call sequence through [`check_sequence`].
#[derive(Clone, Debug)]
pub struct CheckSequenceOutcome {
    pub success: bool,
    pub replayed_entirely: bool,
    pub reason: Option<String>,
    pub calls_count: usize,
    pub reverts: usize,
    pub failure_site: Option<CheckSequenceFailureSite>,
}

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
) -> String {
    let message = match position {
        Some((current, total)) if total > 1 => {
            format!(" [{current}/{total}] Shrink: {label}")
        }
        _ => format!(" Shrink: {label}"),
    };
    if let Some(progress) = progress {
        progress.set_length(config.shrink_run_limit as u64);
        progress.reset();
        progress.set_message(message.clone());
    }
    message
}

/// Live shrink progress display. The progress bar itself is owned by forge's test runner; this
/// type only formats the transient message shown while invariant shrinking is active.
pub(crate) struct ShrinkProgress<'a> {
    progress: Option<&'a ProgressBar>,
    message: String,
    identified_contracts: Option<&'a ContractsByAddress>,
    show_solidity: bool,
    last_draw: Cell<Option<Instant>>,
}

impl<'a> ShrinkProgress<'a> {
    pub(crate) fn new(
        config: &InvariantConfig,
        progress: Option<&'a ProgressBar>,
        label: &str,
        position: Option<(usize, usize)>,
        identified_contracts: Option<&'a ContractsByAddress>,
        show_solidity: bool,
    ) -> Self {
        let message = reset_shrink_progress(config, progress, label, position);
        Self { progress, message, identified_contracts, show_solidity, last_draw: Cell::new(None) }
    }

    fn inc(&self) {
        if let Some(progress) = self.progress {
            progress.inc(1);
        }
    }

    fn update(
        &self,
        calls: &[BasicTxDetails],
        shrinker: &SequenceShrink,
        accumulate_warp_roll: bool,
    ) {
        self.update_with(calls, shrinker, accumulate_warp_roll, false);
    }

    fn finish(
        &self,
        calls: &[BasicTxDetails],
        shrinker: &SequenceShrink,
        accumulate_warp_roll: bool,
    ) {
        self.update_with(calls, shrinker, accumulate_warp_roll, true);
    }

    fn update_with(
        &self,
        calls: &[BasicTxDetails],
        shrinker: &SequenceShrink,
        accumulate_warp_roll: bool,
        force: bool,
    ) {
        let Some(progress) = self.progress else {
            return;
        };
        if progress.is_hidden() {
            return;
        }
        if !force
            && self
                .last_draw
                .get()
                .is_some_and(|last_draw| last_draw.elapsed() < LIVE_SHRINK_PROGRESS_INTERVAL)
        {
            return;
        }

        let message = format_shrink_progress_message(
            &self.message,
            calls,
            shrinker,
            accumulate_warp_roll,
            self.identified_contracts,
            self.show_solidity,
        );
        progress.set_message(message);
        progress.force_draw();
        self.last_draw.set(Some(Instant::now()));
    }
}

fn format_shrink_progress_message(
    phase: &str,
    calls: &[BasicTxDetails],
    shrinker: &SequenceShrink,
    accumulate_warp_roll: bool,
    identified_contracts: Option<&ContractsByAddress>,
    show_solidity: bool,
) -> String {
    let sequence_len = shrinker.included_count();
    let mut message = String::with_capacity(phase.len() + sequence_len.min(32) * 96);
    message.push_str(phase);
    write!(message, "\n\t[Sequence] (shrunk: {sequence_len})").unwrap();

    let displayed =
        shrink_progress_display_calls(calls, shrinker, accumulate_warp_roll, sequence_len);
    let empty_contracts;
    let identified_contracts = if let Some(identified_contracts) = identified_contracts {
        identified_contracts
    } else {
        empty_contracts = ContractsByAddress::default();
        &empty_contracts
    };

    if sequence_len <= LIVE_SHRINK_SEQUENCE_EDGE_CALLS * 2 {
        for tx in &displayed {
            push_shrink_progress_call(&mut message, tx, identified_contracts, show_solidity);
        }
        return message;
    }

    for tx in displayed.iter().take(LIVE_SHRINK_SEQUENCE_EDGE_CALLS) {
        push_shrink_progress_call(&mut message, tx, identified_contracts, show_solidity);
    }
    writeln!(
        message,
        "\n\t\t... {} call(s) omitted ...",
        sequence_len - LIVE_SHRINK_SEQUENCE_EDGE_CALLS * 2
    )
    .unwrap();
    for tx in displayed.iter().skip(displayed.len().saturating_sub(LIVE_SHRINK_SEQUENCE_EDGE_CALLS))
    {
        push_shrink_progress_call(&mut message, tx, identified_contracts, show_solidity);
    }
    message
}

fn shrink_progress_display_calls(
    calls: &[BasicTxDetails],
    shrinker: &SequenceShrink,
    accumulate_warp_roll: bool,
    sequence_len: usize,
) -> Vec<BasicTxDetails> {
    let display_limit = LIVE_SHRINK_SEQUENCE_EDGE_CALLS * 2;
    let mut displayed = Vec::with_capacity(sequence_len.min(display_limit));
    let tail_start = sequence_len.saturating_sub(LIVE_SHRINK_SEQUENCE_EDGE_CALLS);
    let mut included_seen = 0;
    let mut accumulated_warp = U256::ZERO;
    let mut accumulated_roll = U256::ZERO;

    for (idx, call) in calls.iter().enumerate() {
        if accumulate_warp_roll {
            accumulated_warp += call.warp.unwrap_or(U256::ZERO);
            accumulated_roll += call.roll.unwrap_or(U256::ZERO);
        }

        if !shrinker.contains(idx) {
            continue;
        }

        let should_display = sequence_len <= display_limit
            || included_seen < LIVE_SHRINK_SEQUENCE_EDGE_CALLS
            || included_seen >= tail_start;
        if should_display {
            displayed.push(if accumulate_warp_roll {
                apply_warp_roll(call.clone(), accumulated_warp, accumulated_roll)
            } else {
                call.clone()
            });
        }

        included_seen += 1;
        if accumulate_warp_roll {
            accumulated_warp = U256::ZERO;
            accumulated_roll = U256::ZERO;
        }
    }

    displayed
}

fn push_shrink_progress_call(
    message: &mut String,
    tx: &BasicTxDetails,
    identified_contracts: &ContractsByAddress,
    show_solidity: bool,
) {
    let call =
        BaseCounterExample::from_invariant_call(tx, identified_contracts, None, show_solidity)
            .to_string();
    for line in call.lines() {
        message.push('\n');
        message.push_str(line);
    }
}

/// Applies accumulated warp/roll to a call, returning a modified copy.
fn apply_warp_roll(mut result: BasicTxDetails, warp: U256, roll: U256) -> BasicTxDetails {
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
    shrinker: &SequenceShrink,
    accumulate_warp_roll: bool,
) -> Vec<BasicTxDetails> {
    if !accumulate_warp_roll {
        return shrinker.apply(calls);
    }

    shrinker.apply_with_accumulated_delay(calls, |call| (call.warp, call.roll), apply_warp_roll)
}

/// Shared sequence shrinker. Tries to drop each call; `predicate` decides whether the candidate
/// should be accepted, rejected, or skipped without spending a replay attempt.
pub fn shrink_sequence_by_removing<P, S, A>(
    calls_len: usize,
    run: &mut ShrinkRun,
    mut should_stop: S,
    mut on_attempt: A,
    mut predicate: P,
) -> SequenceShrink
where
    P: FnMut(&SequenceShrink) -> Option<bool>,
    S: FnMut() -> bool,
    A: FnMut(),
{
    let mut shrinker = SequenceShrink::new(calls_len);
    let mut call_idx = 0;
    let mut skipped_candidates = 0usize;

    while run.can_try() {
        if should_stop() {
            break;
        }
        let included_count = shrinker.included_count();
        if included_count == 0 || skipped_candidates >= included_count {
            break;
        }

        // Already-removed indices have nothing to drop.
        if !shrinker.contains(call_idx) {
            call_idx = shrinker.next_index(call_idx);
            continue;
        }

        shrinker.remove(call_idx);

        let Some(accepted) = run.try_candidate_decision(|| predicate(&shrinker)) else {
            shrinker.restore(call_idx);
            skipped_candidates += 1;
            call_idx = shrinker.next_index(call_idx);
            continue;
        };

        on_attempt();
        skipped_candidates = 0;
        if accepted {
            if shrinker.included_count() == 1 {
                break;
            }
        } else {
            shrinker.restore(call_idx);
        }

        call_idx = shrinker.next_index(call_idx);
    }

    shrinker
}

/// Shared shrink loop driver. Tries to drop each call; `predicate` returns whether the
/// candidate still triggers the bug.
fn run_shrink_loop<P>(
    config: &InvariantConfig,
    calls: &[BasicTxDetails],
    progress: &ShrinkProgress<'_>,
    accumulate_warp_roll: bool,
    early_exit: &EarlyExit,
    error_policy: ShrinkErrorPolicy,
    mut predicate: P,
) -> SequenceShrink
where
    P: FnMut(&SequenceShrink) -> eyre::Result<bool>,
{
    let mut run = ShrinkRun::new(config.shrink_run_limit as usize);
    progress.update(calls, &SequenceShrink::new(calls.len()), accumulate_warp_roll);
    let shrinker = shrink_sequence_by_removing(
        calls.len(),
        &mut run,
        || early_exit.should_stop(),
        || {
            progress.inc();
        },
        |shrinker| {
            progress.update(calls, shrinker, accumulate_warp_roll);
            match predicate(shrinker) {
                Ok(bug_still_present) => Some(bug_still_present),
                Err(_) => Some(matches!(error_policy, ShrinkErrorPolicy::KeepRemoved)),
            }
        },
    );
    progress.finish(calls, &shrinker, accumulate_warp_roll);
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
    progress: &ShrinkProgress<'_>,
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
        calls,
        progress,
        accumulate_warp_roll,
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
            let bug_still_present = !result.success;
            if bug_still_present {
                last_result = Some(result);
                last_result_matches_shrinker = true;
            }
            Ok(bug_still_present)
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

        let executed = apply_warp_roll(tx.clone(), accumulated_warp, accumulated_roll);
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
/// entirely applied, plus the concrete failure site when replay fails.
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
) -> eyre::Result<CheckSequenceOutcome> {
    let mut calls_executed = 0;
    let mut reverts = 0;
    let early = replay_sequence(
        &mut executor,
        calls,
        &sequence,
        options.accumulate_warp_roll,
        |idx, call_result| {
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
                let site = sequence_call_failure_site(&calls[idx], &call_result);
                return Ok(ReplayDecision::Stop(CheckSequenceOutcome {
                    success: false,
                    replayed_entirely: false,
                    reason: assertion_failure_reason(call_result, options.rd),
                    calls_count: calls_executed,
                    reverts,
                    failure_site: Some(site),
                }));
            }
            if call_result.reverted && options.fail_on_revert {
                if options.expect_assertion_failure {
                    return Ok(ReplayDecision::Stop(CheckSequenceOutcome {
                        success: true,
                        replayed_entirely: false,
                        reason: None,
                        calls_count: calls_executed,
                        reverts,
                        failure_site: None,
                    }));
                }
                let site = sequence_call_failure_site(&calls[idx], &call_result);
                return Ok(ReplayDecision::Stop(CheckSequenceOutcome {
                    success: false,
                    replayed_entirely: false,
                    reason: call_failure_reason(call_result, options.rd),
                    calls_count: calls_executed,
                    reverts,
                    failure_site: Some(site),
                }));
            }
            Ok(ReplayDecision::Continue(call_result))
        },
    )?;
    if let Some(result) = early {
        return Ok(result);
    }

    // Unlike optimization mode we intentionally do not apply trailing warp/roll before the
    // invariant call: those delays would not be representable in the final shrunk sequence.
    let (success, replayed_entirely, reason, failure_site) =
        finish_sequence_check(&executor, test_address, calldata, &options)?;
    Ok(CheckSequenceOutcome {
        success,
        replayed_entirely,
        reason,
        calls_count: calls_executed,
        reverts,
        failure_site,
    })
}

fn finish_sequence_check<FEN: FoundryEvmNetwork>(
    executor: &Executor<FEN>,
    test_address: Address,
    calldata: Bytes,
    options: &CheckSequenceOptions<'_>,
) -> eyre::Result<(bool, bool, Option<String>, Option<CheckSequenceFailureSite>)> {
    let handle_terminal_failure =
        |call_result: RawCallResult<FEN>, site_kind: TerminalFailureSite| {
            let should_ignore_failure = options.expect_assertion_failure
                && !executor.has_global_failure(&call_result.state_changeset)
                && !did_fail_on_assert(&call_result, &call_result.state_changeset);

            if should_ignore_failure {
                return (true, true, None, None);
            }

            let site = terminal_failure_site(site_kind, test_address, &calldata, &call_result);
            let reason = if options.expect_assertion_failure {
                assertion_failure_reason(call_result, options.rd)
            } else {
                call_failure_reason(call_result, options.rd)
            };

            (false, true, reason, Some(site))
        };

    let (invariant_result, mut success) =
        call_invariant_function(executor, test_address, calldata.clone())?;
    if !success {
        return Ok(handle_terminal_failure(invariant_result, TerminalFailureSite::Invariant));
    }

    // Check after invariant result if invariant is success and `afterInvariant` function is
    // declared.
    if success && options.call_after_invariant {
        let (after_invariant_result, after_invariant_success) =
            call_after_invariant_function(executor, test_address)?;
        success = after_invariant_success;
        if !success {
            return Ok(handle_terminal_failure(
                after_invariant_result,
                TerminalFailureSite::AfterInvariant,
            ));
        }
    }

    Ok((success, true, None, None))
}

#[derive(Clone, Copy)]
enum TerminalFailureSite {
    Invariant,
    AfterInvariant,
}

fn sequence_call_failure_site<FEN: FoundryEvmNetwork>(
    call: &BasicTxDetails,
    call_result: &RawCallResult<FEN>,
) -> CheckSequenceFailureSite {
    let target = call_result.reverter.unwrap_or(call.call_details.target);
    let selector = selector_from_calldata(&call.call_details.calldata);
    let fingerprint =
        handler_edge_fingerprint(snapshot_edge_fingerprint(call_result), target, selector);
    CheckSequenceFailureSite::SequenceCall { target, selector, fingerprint }
}

fn terminal_failure_site<FEN: FoundryEvmNetwork>(
    kind: TerminalFailureSite,
    target: Address,
    calldata: &Bytes,
    call_result: &RawCallResult<FEN>,
) -> CheckSequenceFailureSite {
    let target = call_result.reverter.unwrap_or(target);
    let selector = match kind {
        TerminalFailureSite::Invariant => selector_from_calldata(calldata),
        TerminalFailureSite::AfterInvariant => {
            Selector::from(IInvariantTest::afterInvariantCall::SELECTOR)
        }
    };
    let fingerprint =
        handler_edge_fingerprint(snapshot_edge_fingerprint(call_result), target, selector);
    match kind {
        TerminalFailureSite::Invariant => {
            CheckSequenceFailureSite::Invariant { target, selector, fingerprint }
        }
        TerminalFailureSite::AfterInvariant => {
            CheckSequenceFailureSite::AfterInvariant { target, selector, fingerprint }
        }
    }
}

fn selector_from_calldata(calldata: &Bytes) -> Selector {
    let selector: [u8; 4] = calldata.get(..4).and_then(|s| s.try_into().ok()).unwrap_or_default();
    Selector::from(selector)
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
    progress: &ShrinkProgress<'_>,
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

    let replay_failed = Cell::new(false);
    let mut replay_error = None;
    let mut run = ShrinkRun::new(config.shrink_run_limit as usize);
    progress.update(calls, &SequenceShrink::new(calls.len()), true);
    let shrinker = shrink_sequence_by_removing(
        calls.len(),
        &mut run,
        || early_exit.should_stop() || replay_failed.get(),
        || {
            progress.inc();
        },
        |shrinker| {
            progress.update(calls, shrinker, true);
            match check_sequence_value(
                executor.clone(),
                calls,
                shrinker.current().collect(),
                target_address,
                calldata.clone(),
            ) {
                Ok(Some(value)) => Some(value == target_value),
                Ok(None) => Some(false),
                Err(err) => {
                    replay_error = Some(err);
                    replay_failed.set(true);
                    None
                }
            }
        },
    );
    if let Some(err) = replay_error {
        return Err(err);
    }

    progress.finish(calls, &shrinker, true);
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
    progress: &ShrinkProgress<'_>,
    early_exit: &EarlyExit,
) -> eyre::Result<Vec<BasicTxDetails>> {
    if calls.is_empty() {
        return Ok(vec![]);
    }
    let accumulate_warp_roll = config.has_delay();
    let shrinker = run_shrink_loop(
        config,
        calls,
        progress,
        accumulate_warp_roll,
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
pub(crate) fn check_sequence_value<FEN: FoundryEvmNetwork>(
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

            let tx_with_accumulated =
                apply_warp_roll(tx.clone(), accumulated_warp, accumulated_roll);
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
    use super::{
        LIVE_SHRINK_SEQUENCE_EDGE_CALLS, SequenceShrink, ShrinkCandidateKeys, ShrinkErrorPolicy,
        ShrinkProgress, ShrinkRun, build_shrunk_sequence, format_shrink_progress_message,
        run_shrink_loop, shrink_progress_display_calls, shrink_sequence_by_removing,
    };
    use crate::executors::EarlyExit;
    use alloy_primitives::{Address, Bytes, U256};
    use foundry_config::InvariantConfig;
    use foundry_evm_fuzz::{BasicTxDetails, CallDetails};

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

    fn tx_with_calldata(byte: u8) -> BasicTxDetails {
        BasicTxDetails {
            warp: None,
            roll: None,
            sender: Address::ZERO,
            call_details: CallDetails {
                target: Address::ZERO,
                calldata: Bytes::from(vec![byte]),
                value: None,
            },
        }
    }

    fn shrink_progress(config: &InvariantConfig) -> ShrinkProgress<'_> {
        ShrinkProgress::new(config, None, "test", None, None, false)
    }

    #[test]
    fn build_shrunk_sequence_accumulates_removed_delay_into_next_kept_call() {
        let calls = vec![tx(Some(3), Some(5)), tx(Some(7), Some(11)), tx(Some(13), Some(17))];
        let mut shrinker = SequenceShrink::new(calls.len());
        shrinker.remove(0);

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
        let mut shrinker = SequenceShrink::new(calls.len());
        shrinker.remove(1);

        let shrunk = build_shrunk_sequence(&calls, &shrinker, true);

        assert_eq!(shrunk.len(), 1);
        assert_eq!(shrunk[0].warp, Some(U256::from(3)));
        assert_eq!(shrunk[0].roll, Some(U256::from(5)));
    }

    #[test]
    fn shrink_run_counts_attempts_and_accepts() {
        let mut run = ShrinkRun::new(2);

        assert_eq!(run.remaining_attempts(), 2);
        assert!(!run.try_candidate(|| false));
        assert!(run.try_candidate(|| true));

        let mut called_after_limit = false;
        assert!(!run.try_candidate(|| {
            called_after_limit = true;
            true
        }));

        assert!(!called_after_limit);
        let stats = run.finish();
        assert_eq!(stats.attempts, 2);
        assert_eq!(stats.accepted, 1);
    }

    #[test]
    fn shrink_candidate_keys_skip_duplicates() {
        let mut candidates = ShrinkCandidateKeys::new("initial");

        assert!(!candidates.insert("initial"));
        assert!(candidates.insert("first"));
        assert!(!candidates.insert("first"));
        assert!(candidates.insert("second"));
    }

    #[test]
    fn shrink_loop_keep_removed_treats_candidate_error_as_still_failing() {
        let config = InvariantConfig { shrink_run_limit: 1, ..Default::default() };
        let early_exit = EarlyExit::new(false);
        let calls = vec![tx(None, None), tx(None, None)];
        let progress = shrink_progress(&config);

        let shrinker = run_shrink_loop(
            &config,
            &calls,
            &progress,
            false,
            &early_exit,
            ShrinkErrorPolicy::KeepRemoved,
            |_| Err(eyre::eyre!("candidate replay failed")),
        );

        assert_eq!(shrinker.current().collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn shrink_loop_limit_counts_candidate_replays_not_skipped_indices() {
        let config = InvariantConfig { shrink_run_limit: 4, ..Default::default() };
        let early_exit = EarlyExit::new(false);
        let calls = vec![tx(None, None), tx(None, None), tx(None, None)];
        let progress = shrink_progress(&config);
        let mut replay_attempts = 0;

        let shrinker = run_shrink_loop(
            &config,
            &calls,
            &progress,
            false,
            &early_exit,
            ShrinkErrorPolicy::RestoreRemoved,
            |_| {
                replay_attempts += 1;
                Ok(matches!(replay_attempts, 1 | 4))
            },
        );

        assert_eq!(replay_attempts, 4);
        assert_eq!(shrinker.current().collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn shrink_progress_message_renders_current_sequence() {
        let calls = vec![tx_with_calldata(1), tx_with_calldata(2)];
        let shrinker = SequenceShrink::new(calls.len());

        let message = format_shrink_progress_message(
            " Shrink: invariant_live",
            &calls,
            &shrinker,
            false,
            None,
            false,
        );

        assert!(message.contains(" Shrink: invariant_live"));
        assert!(message.contains("[Sequence] (shrunk: 2)"));
        assert!(message.contains("calldata=0x01 args=[]"));
        assert!(message.contains("calldata=0x02 args=[]"));
    }

    #[test]
    fn shrink_progress_message_omits_middle_of_large_sequence() {
        let calls = (0..(LIVE_SHRINK_SEQUENCE_EDGE_CALLS * 2 + 3))
            .map(|idx| tx_with_calldata(idx as u8))
            .collect::<Vec<_>>();
        let shrinker = SequenceShrink::new(calls.len());

        let message = format_shrink_progress_message(
            " Shrink: invariant_live",
            &calls,
            &shrinker,
            false,
            None,
            false,
        );

        assert!(message.contains("[Sequence] (shrunk: 35)"));
        assert!(message.contains("... 3 call(s) omitted ..."));
        assert_eq!(message.matches("sender=").count(), LIVE_SHRINK_SEQUENCE_EDGE_CALLS * 2);
        assert!(message.contains("calldata=0x00 args=[]"));
        assert!(message.contains("calldata=0x22 args=[]"));
    }

    #[test]
    fn shrink_progress_display_calls_accumulates_removed_delay() {
        let calls = vec![tx(Some(3), Some(5)), tx(Some(7), Some(11)), tx(Some(13), Some(17))];
        let mut shrinker = SequenceShrink::new(calls.len());
        shrinker.remove(0);

        let displayed =
            shrink_progress_display_calls(&calls, &shrinker, true, shrinker.included_count());

        assert_eq!(displayed.len(), 2);
        assert_eq!(displayed[0].warp, Some(U256::from(10)));
        assert_eq!(displayed[0].roll, Some(U256::from(16)));
        assert_eq!(displayed[1].warp, Some(U256::from(13)));
        assert_eq!(displayed[1].roll, Some(U256::from(17)));
    }

    #[test]
    fn sequence_shrinker_skips_duplicate_candidates_without_spending_attempts() {
        let mut run = ShrinkRun::new(2);
        let mut seen = Vec::new();

        let shrinker = shrink_sequence_by_removing(
            2,
            &mut run,
            || false,
            || {},
            |shrinker| {
                let candidate = shrinker.current().collect::<Vec<_>>();
                if seen.contains(&candidate) {
                    None
                } else {
                    seen.push(candidate);
                    Some(false)
                }
            },
        );

        assert_eq!(shrinker.current().collect::<Vec<_>>(), vec![0, 1]);
        let stats = run.finish();
        assert_eq!(stats.attempts, 2);
        assert_eq!(stats.accepted, 0);
    }
}
