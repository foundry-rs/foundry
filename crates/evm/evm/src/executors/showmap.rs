//! AFL-`afl-showmap`-style corpus replay.
//!
//! Replays a persisted corpus through a fresh executor and emits one text file
//! per trial (or per corpus entry). Each line has the form `<id>:<count>`:
//!
//! - EVM IDs use the *deterministic* `(bytecode_hash, pc)` derived from the line-coverage `HitMap`
//!   so that IDs are stable across `forge` invocations and meaningful for cross-approach analysis.
//!   Format: `evm_<bytecode_hash[:16]>_<pc:04x>`.
//! - Sancov IDs use the deterministic guard index from the sancov bitmap: `sancov_0x<index:04x>`.
//!
//! Counts are raw saturating-summed hitcounts across the replayed corpus.
//!
//! Output is consumable by tools like `riesentoaster/differential-coverage`.

use crate::{
    executors::{
        Executor,
        corpus::{
            DynamicTargetCtx, WorkerCorpus, register_replay_created, rollback_replay_created,
        },
        corpus_io::read_corpus_tree,
        invariant::{
            call_after_invariant_function, call_invariant_function, did_fail_on_assert, execute_tx,
            snapshot_edge_fingerprint,
        },
    },
    inspectors::EdgeIndexMap,
};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::{Address, B256, Selector, hex};
use eyre::Result;
use foundry_evm_core::{
    constants::{CHEATCODE_ADDRESS, MAGIC_ASSUME},
    evm::FoundryEvmNetwork,
};
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::{BasicTxDetails, invariant::FuzzRunIdentifiedContracts};
use std::{
    collections::HashMap,
    fmt,
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

type EvmShowmap = HashMap<(B256, u32), u64>;

/// Which coverage bitmap(s) to dump.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ShowmapDomain {
    #[default]
    Evm,
    Sancov,
    Both,
}

impl ShowmapDomain {
    pub const fn includes_evm(self) -> bool {
        matches!(self, Self::Evm | Self::Both)
    }
    pub const fn includes_sancov(self) -> bool {
        matches!(self, Self::Sancov | Self::Both)
    }
}

impl fmt::Display for ShowmapDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Evm => f.write_str("evm"),
            Self::Sancov => f.write_str("sancov"),
            Self::Both => f.write_str("both"),
        }
    }
}

/// Per-replay options.
#[derive(Clone, Debug)]
pub struct ShowmapOpts {
    /// Output root directory; emitted files live under `<out_dir>/<approach>/`.
    pub out_dir: PathBuf,
    /// Approach directory name; test identity is folded in here so each
    /// `<approach>/` contains trials of one test (matches `differential-coverage`).
    pub approach: String,
    /// Rerun identifier used as the filename so multiple trials accumulate side-by-side.
    pub trial: String,
    /// Whether to emit one file per corpus entry or one aggregated file.
    pub per_input: bool,
    /// Which bitmap(s) to dump.
    pub domain: ShowmapDomain,
    /// Whether to write showmap files. Disabled by `forge fuzz replay`.
    pub emit_files: bool,
}

/// Stats returned from a single trial replay.
#[derive(Clone, Debug, Default)]
pub struct ShowmapStats {
    /// Number of corpus entries successfully replayed.
    pub corpus_entries: usize,
    /// Number of files written to disk.
    pub showmap_files: usize,
    /// Number of corpus entries skipped because they couldn't be replayed
    /// against the current target (e.g. selector mismatch).
    pub skipped_entries: usize,
    /// Number of corpus entries skipped because they could not be read.
    pub unreadable_entries: usize,
    /// True if sancov coverage was requested. Lets the caller distinguish
    /// "sancov not asked for" from "sancov asked for but produced nothing".
    pub sancov_requested: bool,
    /// True if any non-zero sancov hits were observed across the replay.
    pub sancov_observed: bool,
}

/// Test target metadata needed to replay corpus entries.
pub struct ShowmapReplayTarget<'a> {
    pub fuzzed_function: Option<&'a Function>,
    pub fuzz_fail_on_revert: bool,
    pub fuzzed_contracts: Option<&'a FuzzRunIdentifiedContracts>,
    pub invariant_address: Option<Address>,
    pub invariant_fns: &'a [(&'a Function, bool)],
    pub invariant_replay: InvariantReplayOptions,
    pub dynamic: Option<&'a DynamicTargetCtx<'a>>,
}

/// Invariant replay settings that affect when terminal checks run.
#[derive(Clone, Copy, Debug, Default)]
pub struct InvariantReplayOptions {
    pub check_interval: u32,
    pub call_after_invariant: bool,
}

/// A structured, comparable identity for a failure observed during replay.
///
/// Used by `forge fuzz tmin` to decide whether a minimized candidate still
/// reproduces the *same* failure. It deliberately keys on the failure site (and,
/// for code paths within a single function, an edge-coverage fingerprint) rather
/// than on raw revert bytes, so that:
/// - the same bug reached with different revert arguments is still considered the same failure
///   (avoids false negatives), and
/// - distinct assertion sites that produce identical revert data (e.g. two `Panic(0x01)` paths) are
///   kept distinct (avoids false positives).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayFailure {
    /// A stateless fuzz test call failed. Keyed by selector and code-path fingerprint.
    Fuzz { selector: Selector, fingerprint: Option<B256> },
    /// An invariant handler call hit an assertion or a `fail_on_revert` revert.
    /// Keyed by `(target, selector)` site and code-path fingerprint, mirroring the
    /// campaign's handler-bug deduplication.
    Handler { target: Address, selector: Selector, fingerprint: Option<B256> },
    /// A broken invariant predicate. Keyed by the invariant function name.
    Invariant { name: String },
    /// The `afterInvariant` hook reverted.
    AfterInvariant,
}

impl ReplayFailure {
    /// Whether this failure terminates the run (mirrors the campaign: handler-side
    /// assertion bugs let the campaign keep running, everything else stops it).
    const fn is_terminal(&self) -> bool {
        !matches!(self, Self::Handler { .. })
    }

    /// Whether this failure is a broken invariant predicate (used to gate
    /// `afterInvariant`, which the campaign skips only on predicate breaks).
    const fn is_predicate(&self) -> bool {
        matches!(self, Self::Invariant { .. } | Self::AfterInvariant)
    }
}

impl fmt::Display for ReplayFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fuzz { selector, .. } => write!(f, "fuzz call {selector:?} failed"),
            Self::Handler { target, selector, .. } => {
                write!(f, "handler {selector:?} on {target:?} failed")
            }
            Self::Invariant { name } => write!(f, "invariant `{name}` broken"),
            Self::AfterInvariant => f.write_str("afterInvariant broken"),
        }
    }
}

/// Records `failure` as the representative failure for an observation, preferring
/// terminal failures over non-terminal (handler) ones and keeping the first of
/// each class.
fn record_replay_failure(slot: &mut Option<ReplayFailure>, failure: ReplayFailure) {
    match slot {
        None => *slot = Some(failure),
        Some(existing) if !existing.is_terminal() && failure.is_terminal() => *slot = Some(failure),
        Some(_) => {}
    }
}

/// Facts observed while replaying one candidate for minimization.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplayObservation {
    /// AFL-bucketed EVM edge coverage for the candidate.
    pub evm_edges: Vec<u8>,
    /// AFL-bucketed native sancov edge coverage for the candidate.
    pub sancov_edges: Vec<u8>,
    /// Comparable failure identity, if replaying this candidate fails.
    pub failure: Option<ReplayFailure>,
    /// Number of replayable transactions executed.
    pub replayed: usize,
    /// Number of transactions skipped because they do not target this fuzz/invariant
    /// context, or were rejected via `vm.assume`.
    pub skipped: usize,
}

impl ReplayObservation {
    pub fn has_coverage(&self) -> bool {
        self.evm_edges.iter().any(|&edge| edge != 0)
            || self.sancov_edges.iter().any(|&edge| edge != 0)
    }

    pub fn merge_edge_coverage(&mut self, other: &Self) {
        merge_edge_vec(&mut self.evm_edges, &other.evm_edges);
        merge_edge_vec(&mut self.sancov_edges, &other.sancov_edges);
    }
}

fn merge_edge_vec(dst: &mut Vec<u8>, src: &[u8]) {
    if dst.len() < src.len() {
        dst.resize(src.len(), 0);
    }
    for (dst, src) in dst.iter_mut().zip(src) {
        *dst = (*dst).max(*src);
    }
}

/// Replay every corpus entry under `corpus_dir` and emit showmap files.
///
/// `fuzzed_function` is set for stateless fuzz tests; `fuzzed_contracts` is set
/// for invariant tests (txs are committed between calls in that case).
/// `dynamic` lets invariant replay register contracts deployed mid-sequence so
/// follow-up calls into them aren't dropped.
pub fn replay_corpus_to_showmap<FEN: FoundryEvmNetwork>(
    executor: &Executor<FEN>,
    corpus_dir: &Path,
    target: ShowmapReplayTarget<'_>,
    opts: &ShowmapOpts,
) -> Result<ShowmapStats> {
    let entries = read_corpus_tree(corpus_dir)?;

    let approach_dir = opts.out_dir.join(&opts.approach);
    if opts.emit_files {
        foundry_common::fs::create_dir_all(&approach_dir)?;
    }

    let mut stats =
        ShowmapStats { sancov_requested: opts.domain.includes_sancov(), ..Default::default() };
    // Reused per call. In aggregate mode it accumulates across all entries; in per-input mode it
    // is cleared after each entry's file is written.
    let mut evm_buf = EvmShowmap::new();
    let mut san_buf: Vec<u64> = Vec::new();

    for entry in entries {
        let tx_seq = match entry.read_tx_seq() {
            Ok(seq) if !seq.is_empty() => seq,
            Ok(_) => continue,
            Err(err) => {
                debug!(target: "showmap", %err, ?entry.path, "failed to read corpus entry");
                stats.unreadable_entries += 1;
                stats.skipped_entries += 1;
                continue;
            }
        };

        let mut had_replayable = false;
        let mut executor = executor.clone();
        // Targets deployed during this entry, cleared after the entry.
        let mut created: Vec<Address> = Vec::new();
        let fail_on_revert = target.invariant_fns.iter().any(|(_, fail_on_revert)| *fail_on_revert);
        // Number of committed (non-`vm.assume`) calls, used to gate invariant checks.
        let mut accepted = 0usize;
        let mut entry_failure: Option<ReplayFailure> = None;
        for tx in &tx_seq {
            if !WorkerCorpus::can_replay_tx(tx, target.fuzzed_function, target.fuzzed_contracts) {
                continue;
            }
            had_replayable = true;

            let mut call_result = execute_tx(&mut executor, tx)?;
            // Snapshot the edge fingerprint before any coverage merge zeroes the buffer.
            let fingerprint = snapshot_edge_fingerprint(&call_result);
            // Coverage-collection asymmetry across calls within a stateful sequence:
            // - line_coverage is per-call: `Executor::call_raw` returns a fresh HitMap each time,
            //   so we can simply accumulate it.
            // - sancov_coverage is the inspector's shared `Vec<u8>` buffer that keeps growing
            //   across calls, so after consuming it we zero it out to avoid double-counting on the
            //   next iteration.
            if opts.domain.includes_evm() {
                accumulate_evm(&mut evm_buf, call_result.line_coverage.as_ref());
            }
            if opts.domain.includes_sancov() {
                accumulate_sancov(&mut san_buf, call_result.sancov_coverage.as_deref());
                if let Some(buf) = call_result.sancov_coverage.as_mut() {
                    buf.fill(0);
                }
            }

            // `vm.assume` rejects are discarded by the campaign: coverage is still
            // collected (above) but the call is neither committed nor checked.
            if call_result.result.as_ref() == MAGIC_ASSUME {
                continue;
            }

            register_replay_created(
                &call_result.state_changeset,
                target.dynamic,
                target.fuzzed_contracts,
                &mut created,
            );

            let target_addr = tx.call_details.target;
            let selector =
                tx.call_details.calldata.get(..4).map(Selector::from_slice).unwrap_or_default();

            // Stateful tests need the tx committed so subsequent calls see its effects.
            if target.fuzzed_contracts.is_some() {
                accepted += 1;
                if !opts.emit_files
                    && let Some(failure) = invariant_handler_failure(
                        target_addr,
                        selector,
                        did_fail_on_assert(&call_result, &call_result.state_changeset),
                        fail_on_revert,
                        &call_result,
                        fingerprint,
                    )
                {
                    entry_failure = Some(failure);
                    break;
                }
                executor.commit(&mut call_result);
                if !opts.emit_files
                    && should_check_invariant(accepted, target.invariant_replay.check_interval)
                    && let Some(address) = target.invariant_address
                    && let Some(failure) =
                        broken_invariant(&executor, address, target.invariant_fns)?
                {
                    entry_failure = Some(failure);
                    break;
                }
            } else if !opts.emit_files {
                let success = if !target.fuzz_fail_on_revert
                    && call_result.reverter.is_some_and(|reverter| {
                        reverter != target_addr && reverter != CHEATCODE_ADDRESS
                    }) {
                    true
                } else {
                    executor.is_raw_call_mut_success(target_addr, &mut call_result, false)
                };
                if !success {
                    entry_failure = Some(ReplayFailure::Fuzz { selector, fingerprint });
                    break;
                }
            }
        }
        // Final invariant + afterInvariant checks (replay mode only): mirror the
        // campaign's "always check on the last call", and run afterInvariant unless a
        // predicate already broke.
        if !opts.emit_files
            && entry_failure.is_none()
            && accepted > 0
            && target.fuzzed_contracts.is_some()
            && let Some(address) = target.invariant_address
        {
            if let Some(failure) = broken_invariant(&executor, address, target.invariant_fns)? {
                entry_failure = Some(failure);
            } else if target.invariant_replay.call_after_invariant
                && let Some(failure) = broken_after_invariant(&executor, address)?
            {
                entry_failure = Some(failure);
            }
        }
        if let Some(failure) = entry_failure {
            rollback_replay_created(target.fuzzed_contracts, created);
            return Err(eyre::eyre!(
                "corpus entry {} failed during replay: {failure}",
                entry.path.display()
            ));
        }
        rollback_replay_created(target.fuzzed_contracts, created);

        if !had_replayable {
            stats.skipped_entries += 1;
            continue;
        }
        stats.corpus_entries += 1;
        if !stats.sancov_observed && san_buf.iter().any(|&x| x != 0) {
            stats.sancov_observed = true;
        }

        if opts.emit_files && opts.per_input {
            // <trial>__<uuid>-<ts>.txt
            let stem = format!("{}__{}-{}", opts.trial, entry.uuid, entry.timestamp);
            stats.showmap_files +=
                write_showmap_file(&approach_dir.join(format!("{stem}.txt")), &evm_buf, &san_buf)?;
            // Reset for the next entry; preserves capacity so we don't reallocate.
            evm_buf.clear();
            san_buf.fill(0);
        }
    }

    if opts.emit_files && !opts.per_input {
        // <trial>.txt
        stats.showmap_files += write_showmap_file(
            &approach_dir.join(format!("{}.txt", opts.trial)),
            &evm_buf,
            &san_buf,
        )?;
    }

    Ok(stats)
}

/// Replay one in-memory candidate and return edge/failure observations.
pub struct MinimizationReplayInput<'a> {
    pub sequence: &'a [BasicTxDetails],
    pub evm_edge_indices: &'a mut EdgeIndexMap,
}

pub fn replay_sequence_for_minimization<FEN: FoundryEvmNetwork>(
    executor: &Executor<FEN>,
    input: MinimizationReplayInput<'_>,
    target: ShowmapReplayTarget<'_>,
) -> Result<ReplayObservation> {
    let mut observation = ReplayObservation::default();
    let mut executor = executor.clone();
    executor.inspector_mut().collect_edge_coverage(true);
    executor.inspector_mut().collect_sancov_edges(true);

    let fail_on_revert = target.invariant_fns.iter().any(|(_, fail_on_revert)| *fail_on_revert);
    let mut created = Vec::new();
    // Number of committed (non-`vm.assume`) calls, used to gate invariant checks.
    let mut accepted = 0usize;
    for tx in input.sequence {
        if !WorkerCorpus::can_replay_tx(tx, target.fuzzed_function, target.fuzzed_contracts) {
            observation.skipped += 1;
            continue;
        }
        let mut call_result = execute_tx(&mut executor, tx)?;
        let target_addr = tx.call_details.target;
        let selector =
            tx.call_details.calldata.get(..4).map(Selector::from_slice).unwrap_or_default();
        // Snapshot the edge fingerprint before `merge_all_coverage` zeroes the buffer.
        let fingerprint = snapshot_edge_fingerprint(&call_result);
        call_result.merge_all_coverage(
            &mut observation.evm_edges,
            input.evm_edge_indices,
            &mut observation.sancov_edges,
        );

        // `vm.assume` rejects are discarded by the campaign: coverage is still merged
        // (above) but the call is neither committed, checked, nor treated as a failure.
        if call_result.result.as_ref() == MAGIC_ASSUME {
            observation.skipped += 1;
            continue;
        }
        observation.replayed += 1;

        register_replay_created(
            &call_result.state_changeset,
            target.dynamic,
            target.fuzzed_contracts,
            &mut created,
        );

        if target.fuzzed_contracts.is_some() {
            accepted += 1;
            if let Some(failure) = invariant_handler_failure(
                target_addr,
                selector,
                did_fail_on_assert(&call_result, &call_result.state_changeset),
                fail_on_revert,
                &call_result,
                fingerprint,
            ) {
                record_replay_failure(&mut observation.failure, failure);
            }
            executor.commit(&mut call_result);
            // Skip the predicate check only when a predicate already broke; handler
            // bugs are non-terminal and the campaign keeps checking.
            if !observation.failure.as_ref().is_some_and(ReplayFailure::is_predicate)
                && should_check_invariant(accepted, target.invariant_replay.check_interval)
                && let Some(address) = target.invariant_address
                && let Some(failure) = broken_invariant(&executor, address, target.invariant_fns)?
            {
                record_replay_failure(&mut observation.failure, failure);
            }
        } else {
            let success = if !target.fuzz_fail_on_revert
                && call_result.reverter.is_some_and(|reverter| {
                    reverter != target_addr && reverter != CHEATCODE_ADDRESS
                }) {
                true
            } else {
                executor.is_raw_call_mut_success(target_addr, &mut call_result, false)
            };
            if !success {
                record_replay_failure(
                    &mut observation.failure,
                    ReplayFailure::Fuzz { selector, fingerprint },
                );
            }
        }

        // The campaign stops a run on a terminal failure (broken predicate / failed
        // fuzz call); handler-side assertion bugs let it continue.
        if observation.failure.as_ref().is_some_and(ReplayFailure::is_terminal) {
            break;
        }
    }
    // Mirror the campaign's "always check on the last call" by running a final
    // predicate check, unless a predicate already broke.
    if !observation.failure.as_ref().is_some_and(ReplayFailure::is_predicate)
        && accepted > 0
        && target.fuzzed_contracts.is_some()
        && let Some(address) = target.invariant_address
        && let Some(failure) = broken_invariant(&executor, address, target.invariant_fns)?
    {
        record_replay_failure(&mut observation.failure, failure);
    }
    // `afterInvariant` runs unless a predicate broke (handler bugs don't suppress it).
    if !observation.failure.as_ref().is_some_and(ReplayFailure::is_predicate)
        && accepted > 0
        && target.fuzzed_contracts.is_some()
        && target.invariant_replay.call_after_invariant
        && let Some(address) = target.invariant_address
        && let Some(failure) = broken_after_invariant(&executor, address)?
    {
        record_replay_failure(&mut observation.failure, failure);
    }
    rollback_replay_created(target.fuzzed_contracts, created);
    Ok(observation)
}

/// Returns a [`ReplayFailure::Handler`] if a handler call should be treated as a
/// bug, mirroring the campaign: assertion failures always count, plain reverts only
/// count under `fail_on_revert` and are never counted for `vm.assume` rejects.
fn invariant_handler_failure<FEN: FoundryEvmNetwork>(
    target: Address,
    selector: Selector,
    assertion_failure: bool,
    fail_on_revert: bool,
    call_result: &crate::executors::RawCallResult<FEN>,
    fingerprint: Option<B256>,
) -> Option<ReplayFailure> {
    let failed = assertion_failure
        || (fail_on_revert && call_result.reverted && call_result.result.as_ref() != MAGIC_ASSUME);
    failed.then_some(ReplayFailure::Handler { target, selector, fingerprint })
}

fn broken_invariant<FEN: FoundryEvmNetwork>(
    executor: &Executor<FEN>,
    invariant_address: Address,
    invariant_fns: &[(&Function, bool)],
) -> Result<Option<ReplayFailure>> {
    for (invariant, _) in invariant_fns {
        let (_, success) = call_invariant_function(
            executor,
            invariant_address,
            invariant.abi_encode_input(&[])?.into(),
        )?;
        if !success {
            return Ok(Some(ReplayFailure::Invariant { name: invariant.name.clone() }));
        }
    }
    Ok(None)
}

fn broken_after_invariant<FEN: FoundryEvmNetwork>(
    executor: &Executor<FEN>,
    invariant_address: Address,
) -> Result<Option<ReplayFailure>> {
    let (_, success) = call_after_invariant_function(executor, invariant_address)?;
    Ok((!success).then_some(ReplayFailure::AfterInvariant))
}

/// Whether the invariant predicate should be evaluated after the `accepted`-th
/// committed (non-`vm.assume`) call.
///
/// Mirrors the campaign: with `check_interval == 0` only the final call is checked
/// (callers additionally perform a final check after the sequence ends); with
/// `check_interval == 1` every call is checked; otherwise every N-th call.
fn should_check_invariant(accepted: usize, check_interval: u32) -> bool {
    debug_assert!(accepted > 0);
    check_interval == 1 || (check_interval > 1 && accepted.is_multiple_of(check_interval as usize))
}

/// Saturating-add per-(bytecode, pc) hits from a `HitMaps` snapshot into `dst`.
fn accumulate_evm(dst: &mut EvmShowmap, src: Option<&HitMaps>) {
    let Some(maps) = src else { return };
    for (hash, hitmap) in maps.iter() {
        for (pc, hits) in hitmap.iter() {
            let slot = dst.entry((*hash, pc)).or_default();
            *slot = slot.saturating_add(hits as u64);
        }
    }
}

/// Saturating-add `src` (u8 raw counts) into `dst` (u64 aggregated counts).
fn accumulate_sancov(dst: &mut Vec<u64>, src: Option<&[u8]>) {
    let Some(src) = src else { return };
    if dst.len() < src.len() {
        dst.resize(src.len(), 0);
    }
    for (d, &s) in dst.iter_mut().zip(src) {
        if s != 0 {
            *d = d.saturating_add(s as u64);
        }
    }
}

/// Write a single showmap file. Returns 1 if a file was written, 0 if skipped
/// (no nonzero entries).
fn write_showmap_file(path: &Path, evm: &EvmShowmap, san: &[u64]) -> Result<usize> {
    // Pre-check so we don't create empty files.
    let has_evm = evm.values().any(|&c| c != 0);
    let has_san = san.iter().any(|&c| c != 0);
    if !has_evm && !has_san {
        return Ok(0);
    }
    let mut w = BufWriter::new(File::create_new(path).map_err(|err| {
        eyre::eyre!(
            "failed to create showmap file {}: {err}; pick a different --showmap-trial or remove \
             the existing file",
            path.display()
        )
    })?);
    write_evm(&mut w, evm)?;
    write_sancov(&mut w, san)?;
    w.flush()?;
    Ok(1)
}

/// Each EVM ID is `evm_<bytecode_hash[:16hex]>_<pc:04x>`. The 16-hex prefix
/// (64 bits) of the keccak256 bytecode hash makes IDs deterministic across
/// processes while keeping line lengths short.
fn write_evm<W: Write>(out: &mut W, evm: &EvmShowmap) -> std::io::Result<()> {
    let mut entries = evm.iter().filter(|(_, count)| **count != 0).collect::<Vec<_>>();
    entries.sort_unstable_by_key(|((hash, pc), _)| (*hash, *pc));

    for ((hash, pc), count) in entries {
        let h = hex::encode(&hash.as_slice()[..8]);
        writeln!(out, "evm_{h}_{pc:04x}:{count}")?;
    }
    Ok(())
}

fn write_sancov<W: Write>(out: &mut W, bitmap: &[u64]) -> std::io::Result<()> {
    for (idx, &count) in bitmap.iter().enumerate() {
        if count != 0 {
            // Underscore (not `:`) between prefix and id keeps the showmap
            // `<id>:<count>` parser unambiguous.
            writeln!(out, "sancov_0x{idx:04x}:{count}")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executors::{RawCallResult, corpus_io::canonical_replay_dirs};
    use foundry_evm_core::evm::EthEvmNetwork;
    use revm::interpreter::InstructionResult;
    use uuid::Uuid;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("foundry-showmap-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn accumulate_sancov_resizes_and_saturating_adds() {
        let mut dst: Vec<u64> = vec![10];
        accumulate_sancov(&mut dst, Some(&[1u8, 2, 3]));
        assert_eq!(dst, vec![11, 2, 3]);
    }

    #[test]
    fn write_evm_emits_only_nonzero_deterministic_ids() {
        let mut buf: Vec<u8> = Vec::new();
        let h = B256::with_last_byte(0xab);
        let mut evm = EvmShowmap::new();
        evm.insert((h, 1u32), 0u64); // skipped (count=0)
        evm.insert((h, 0x2au32), 3u64);
        write_evm(&mut buf, &evm).unwrap();
        let h_hex = hex::encode(&h.as_slice()[..8]);
        assert_eq!(String::from_utf8(buf).unwrap(), format!("evm_{h_hex}_002a:3\n"));
    }

    #[test]
    fn write_sancov_emits_only_nonzero_hex_ids() {
        let mut buf: Vec<u8> = Vec::new();
        write_sancov(&mut buf, &[0, 3, 0, 1]).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "sancov_0x0001:3\nsancov_0x0003:1\n");
    }

    #[test]
    fn write_showmap_file_skips_when_empty() {
        let dir = temp_dir();
        let path = dir.join("trial.txt");
        let written = write_showmap_file(&path, &EvmShowmap::new(), &[]).unwrap();
        assert_eq!(written, 0);
        assert!(!path.exists());
    }

    #[test]
    fn write_showmap_file_writes_combined_domains() {
        let dir = temp_dir();
        let path = dir.join("trial.txt");
        let h = B256::with_last_byte(0xff);
        let mut evm = EvmShowmap::new();
        evm.insert((h, 7u32), 5u64);
        let written = write_showmap_file(&path, &evm, &[2]).unwrap();
        assert_eq!(written, 1);
        let body = std::fs::read_to_string(&path).unwrap();
        let h_hex = hex::encode(&h.as_slice()[..8]);
        assert_eq!(body, format!("evm_{h_hex}_0007:5\nsancov_0x0000:2\n"));
    }

    #[test]
    fn write_showmap_file_does_not_overwrite_existing_file() {
        let dir = temp_dir();
        let path = dir.join("trial.txt");
        std::fs::write(&path, "keep me").unwrap();
        let h = B256::with_last_byte(0xff);
        let mut evm = EvmShowmap::new();
        evm.insert((h, 7u32), 5u64);
        let err = write_showmap_file(&path, &evm, &[]).unwrap_err();
        assert!(err.to_string().contains("File exists"), "{err:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep me");
    }

    #[test]
    fn should_check_invariant_matches_campaign_intervals() {
        // check_interval == 1: every accepted call.
        assert!(should_check_invariant(1, 1));
        assert!(should_check_invariant(2, 1));
        // check_interval == 0: never inline (callers do a final check instead).
        assert!(!should_check_invariant(1, 0));
        assert!(!should_check_invariant(5, 0));
        // check_interval == N: every N-th accepted call.
        assert!(!should_check_invariant(1, 3));
        assert!(!should_check_invariant(2, 3));
        assert!(should_check_invariant(3, 3));
        assert!(should_check_invariant(6, 3));
    }

    #[test]
    fn record_replay_failure_prefers_terminal_then_keeps_first() {
        let handler = ReplayFailure::Handler {
            target: Address::ZERO,
            selector: Selector::from([1, 2, 3, 4]),
            fingerprint: None,
        };
        let predicate = ReplayFailure::Invariant { name: "inv".to_string() };

        // Handler recorded first, then upgraded by a terminal predicate failure.
        let mut slot = None;
        record_replay_failure(&mut slot, handler.clone());
        assert_eq!(slot, Some(handler.clone()));
        record_replay_failure(&mut slot, predicate.clone());
        assert_eq!(slot, Some(predicate.clone()));
        // A later handler failure does not displace an existing terminal failure.
        record_replay_failure(&mut slot, handler.clone());
        assert_eq!(slot, Some(predicate.clone()));

        // First terminal failure wins over a subsequent terminal failure.
        let mut slot = None;
        record_replay_failure(&mut slot, predicate.clone());
        record_replay_failure(&mut slot, ReplayFailure::AfterInvariant);
        assert_eq!(slot, Some(predicate));

        assert!(!handler.is_terminal());
        assert!(ReplayFailure::AfterInvariant.is_predicate());
    }

    #[test]
    fn replay_failure_handler_distinguishes_code_paths_by_fingerprint() {
        let site = (Address::with_last_byte(7), Selector::from([9, 9, 9, 9]));
        let a = ReplayFailure::Handler {
            target: site.0,
            selector: site.1,
            fingerprint: Some(B256::with_last_byte(1)),
        };
        let b = ReplayFailure::Handler {
            target: site.0,
            selector: site.1,
            fingerprint: Some(B256::with_last_byte(2)),
        };
        // Same site, different code path => distinct identities (no false positive).
        assert_ne!(a, b);
    }

    #[test]
    fn invariant_handler_failure_ignores_plain_revert() {
        let call_result = RawCallResult::<EthEvmNetwork> {
            reverted: true,
            exit_reason: Some(InstructionResult::Revert),
            ..Default::default()
        };
        let failure = invariant_handler_failure(
            Address::with_last_byte(1),
            Selector::from([0xaa, 0xbb, 0xcc, 0xdd]),
            did_fail_on_assert(&call_result, &call_result.state_changeset),
            false,
            &call_result,
            None,
        );
        assert_eq!(failure, None);
    }

    #[test]
    fn invariant_handler_failure_reports_fail_on_revert() {
        let call_result = RawCallResult::<EthEvmNetwork> {
            reverted: true,
            exit_reason: Some(InstructionResult::Revert),
            ..Default::default()
        };
        let failure = invariant_handler_failure(
            Address::with_last_byte(1),
            Selector::from([0xaa, 0xbb, 0xcc, 0xdd]),
            did_fail_on_assert(&call_result, &call_result.state_changeset),
            true,
            &call_result,
            None,
        );
        assert!(failure.is_some());
    }

    #[test]
    fn canonical_replay_dirs_collects_all_workers() {
        let dir = temp_dir();
        let w0 = dir.join("worker0").join("corpus");
        let w1 = dir.join("worker1").join("corpus");
        std::fs::create_dir_all(&w0).unwrap();
        std::fs::create_dir_all(&w1).unwrap();
        assert_eq!(canonical_replay_dirs(&dir), vec![w0, w1]);
    }

    #[test]
    fn canonical_replay_dirs_falls_back_when_no_workers() {
        let dir = temp_dir();
        assert_eq!(canonical_replay_dirs(&dir), vec![dir]);
    }
}
