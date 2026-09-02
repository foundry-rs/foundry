//! Corpus management for parallel fuzzing with coverage-guided mutation.
//!
//! This module implements a corpus-based fuzzing system that stores, mutates, and shares
//! transaction sequences across multiple fuzzing workers. Each corpus entry represents a
//! sequence of transactions that has produced interesting coverage, and can be mutated to
//! discover new execution paths.
//!
//! ## File System Structure
//!
//! The corpus is organized on disk as follows:
//!
//! ```text
//! <corpus_dir>/
//! ├── worker0/                  # Master (worker 0) directory
//! │   ├── corpus/               # Master's corpus entries
//! │   │   ├── <uuid>-<timestamp>.json          # Corpus entry (if small)
//! │   │   ├── <uuid>-<timestamp>.json.gz       # Corpus entry (if large, compressed)
//! │   └── sync/                 # Directory where other workers export new findings
//! │       └── <uuid>-<timestamp>.json          # New entries from other workers
//! └── workerN/                  # Worker N's directory
//!     ├── corpus/               # Worker N's local corpus
//!     │   └── ...
//!     └── sync/                 # Worker 2's sync directory
//!         └── ...
//! ```
//!
//! ## Workflow
//!
//! - Each worker maintains its own local corpus with entries stored as JSON files
//! - Workers export new interesting entries to the master's sync directory via hard links
//! - The master (worker0) imports new entries from its sync directory and exports them to all the
//!   other workers
//! - Workers sync with the master to receive new corpus entries from other workers
//! - This all happens periodically, there is no clear order in which workers export or import
//!   entries since it doesn't matter as long as the corpus eventually syncs across all workers

use super::corpus_io::{
    CorpusDirEntry, canonical_replay_dirs, read_corpus_dir, read_corpus_dir_strict,
};
use crate::{
    executors::{Executor, RawCallResult, invariant::execute_tx},
    inspectors::{CmpOperands, EdgeIndexMap, MAX_EDGE_COUNT},
};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes, I256, U256};
use eyre::{Result, eyre};
use foundry_common::{ContractsByAddress, ContractsByArtifact, TestFunctionExt, sh_warn};
use foundry_config::FuzzCorpusConfig;
use foundry_evm_core::{constants::CALLER, evm::FoundryEvmNetwork, utils::StateChangeset};
#[cfg(test)]
use foundry_evm_fuzz::strategies::EvmFuzzState;
#[cfg(test)]
use foundry_evm_fuzz::strategies::TxGenerator;
use foundry_evm_fuzz::{
    BasicTxDetails, CallDetails, ObservedCall,
    invariant::{
        ArtifactFilters, FuzzRunIdentifiedContracts, InvariantContract, TargetedContracts,
    },
    sequence::{ComparisonHint, CorpusEntryView, SequenceGenerator, SequencePlan},
};
#[cfg(test)]
use proptest::prelude::Strategy;
use proptest::test_runner::TestRunner;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fmt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const WORKER: &str = "worker";
const CORPUS_DIR: &str = "corpus";
const SYNC_DIR: &str = "sync";
const OPTIMIZATION_BEST_FILE: &str = "optimization_best.json";

const FAVORABILITY_THRESHOLD: f64 = 0.3;

/// Threshold for compressing corpus entries.
/// 4KiB is usually the minimum file size on popular file systems.
const GZIP_THRESHOLD: usize = 4 * 1024;

/// Persisted optimization state: the best value found and the sequence that produced it.
#[derive(Clone, Serialize, Deserialize)]
struct OptimizationState {
    best_value: I256,
    best_sequence: Vec<BasicTxDetails>,
}

/// Holds Corpus information.
#[derive(Clone, Serialize)]
struct CorpusEntry {
    // Unique corpus identifier.
    uuid: Uuid,
    // Total mutations of corpus as primary source.
    total_mutations: usize,
    // New coverage found as a result of mutating this corpus.
    new_finds_produced: usize,
    // Corpus call sequence.
    #[serde(skip_serializing)]
    tx_seq: Vec<BasicTxDetails>,
    // Per-call EVM comparison operands observed while executing this corpus entry.
    // Parallel to `tx_seq`. Empty inner vec means "no cmp data for this call".
    #[serde(skip_serializing)]
    cmp_seq: Vec<Vec<ComparisonHint>>,
    // Whether this corpus is favored, i.e. producing new finds more often than
    // `FAVORABILITY_THRESHOLD`.
    is_favored: bool,
    /// Timestamp of when this entry was written to disk in seconds.
    #[serde(skip_serializing)]
    timestamp: u64,
    /// Original filename for an entry imported from another worker.
    #[serde(skip_serializing)]
    persisted_file_name: Option<String>,
}

impl CorpusEntry {
    /// Creates a corpus entry with a new UUID.
    pub fn new(tx_seq: Vec<BasicTxDetails>) -> Self {
        Self::new_with_cmp(tx_seq, Vec::new(), Uuid::new_v4())
    }

    /// Creates a corpus entry with the given UUID and per-call cmp operand log.
    pub fn new_with_cmp(
        tx_seq: Vec<BasicTxDetails>,
        cmp_seq: Vec<Vec<ComparisonHint>>,
        uuid: Uuid,
    ) -> Self {
        Self {
            uuid,
            total_mutations: 0,
            new_finds_produced: 0,
            tx_seq,
            cmp_seq,
            is_favored: false,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time went backwards")
                .as_secs(),
            persisted_file_name: None,
        }
    }

    fn write_to_disk_in(&self, dir: &Path, can_gzip: bool) -> foundry_common::fs::Result<PathBuf> {
        let should_gzip = self.should_gzip(can_gzip);
        let file_name = self.file_name(should_gzip);
        let path = dir.join(&file_name);
        let temp_path = dir.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));

        let write_result = if should_gzip {
            foundry_common::fs::write_json_gzip_file(&temp_path, &self.tx_seq)
        } else {
            foundry_common::fs::write_json_file(&temp_path, &self.tx_seq)
        };
        if let Err(err) = write_result {
            let _ = foundry_common::fs::remove_file(&temp_path);
            return Err(err);
        }

        if let Err(err) = std::fs::rename(&temp_path, &path) {
            let _ = foundry_common::fs::remove_file(&temp_path);
            return Err(foundry_common::errors::FsPathError::write(err, &path));
        }

        Ok(path)
    }

    fn file_name(&self, gzip: bool) -> String {
        if let Some(name) = &self.persisted_file_name {
            return name.clone();
        }
        let ext = if gzip { ".json.gz" } else { ".json" };
        format!("{}-{}{ext}", self.uuid, self.timestamp)
    }

    fn should_gzip(&self, can_gzip: bool) -> bool {
        if !can_gzip {
            return false;
        }
        let size: usize = self.tx_seq.iter().map(|tx| tx.estimate_serialized_size()).sum();
        size > GZIP_THRESHOLD
    }
}

/// Persists one call sequence as a corpus seed in the canonical worker0 corpus directory.
pub fn persist_corpus_seed(
    config: &FuzzCorpusConfig,
    tx_seq: Vec<BasicTxDetails>,
) -> foundry_common::fs::Result<Option<PathBuf>> {
    let Some(root) = &config.corpus_dir else {
        return Ok(None);
    };
    for dir in canonical_replay_dirs(root) {
        for entry in read_corpus_dir(&dir) {
            match entry.read_tx_seq() {
                Ok(existing) if same_tx_sequence(&existing, &tx_seq) => {
                    return Ok(Some(entry.path));
                }
                Ok(_) => {}
                Err(err) => debug!(%err, path = ?entry.path, "failed to read corpus seed"),
            }
        }
    }
    let corpus_dir = root.join(format!("{WORKER}0")).join(CORPUS_DIR);
    foundry_common::fs::create_dir_all(&corpus_dir)?;
    CorpusEntry::new(tx_seq).write_to_disk_in(&corpus_dir, config.corpus_gzip).map(Some)
}

fn same_tx_sequence(left: &[BasicTxDetails], right: &[BasicTxDetails]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.warp == right.warp
                && left.roll == right.roll
                && left.sender == right.sender
                && left.call_details.target == right.call_details.target
                && left.call_details.calldata == right.call_details.calldata
                && left.call_details.value == right.call_details.value
        })
}

fn link_corpus_file(from: &Path, to: &Path) -> bool {
    match std::fs::hard_link(from, to) {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            if !std::fs::symlink_metadata(to).is_ok_and(|metadata| metadata.file_type().is_file()) {
                debug!(target: "corpus", from=?from, to=?to, "corpus destination is not a regular file");
                return false;
            }
            let entry = |path: &Path| CorpusDirEntry {
                path: path.to_path_buf(),
                uuid: Uuid::nil(),
                timestamp: 0,
            };
            let matches = entry(from)
                .read_tx_seq()
                .ok()
                .zip(entry(to).read_tx_seq().ok())
                .is_some_and(|(from, to)| same_tx_sequence(&from, &to));
            if !matches {
                debug!(target: "corpus", from=?from, to=?to, "conflicting corpus file already exists");
            }
            matches
        }
        Err(err) => {
            debug!(target: "corpus", %err, from=?from, to=?to, "failed to link corpus file");
            false
        }
    }
}

fn accept_synced_corpus_file(
    entry: &CorpusDirEntry,
    tx_seq: &[BasicTxDetails],
    corpus_path: &Path,
) -> bool {
    if corpus_path.is_file() {
        let existing = CorpusDirEntry {
            path: corpus_path.to_path_buf(),
            uuid: entry.uuid,
            timestamp: entry.timestamp,
        };
        if existing.read_tx_seq().is_ok_and(|existing| same_tx_sequence(&existing, tx_seq)) {
            if let Err(err) = std::fs::remove_file(&entry.path) {
                debug!(target: "corpus", %err, "failed to remove synced corpus link {}", entry.path.display());
                return false;
            }
            return true;
        }

        warn!(target: "corpus", "not overwriting conflicting corpus file {}", corpus_path.display());
        let quarantine_path =
            entry.path.with_file_name(format!("{}.{}.invalid", entry.name(), Uuid::new_v4()));
        if let Err(err) = std::fs::rename(&entry.path, &quarantine_path) {
            debug!(target: "corpus", %err, "failed to quarantine conflicting corpus file {}", entry.path.display());
        }
        return false;
    }

    if let Err(err) = std::fs::rename(&entry.path, corpus_path) {
        debug!(target: "corpus", %err, "failed to move synced corpus from {:?} to {corpus_path:?} dir", entry.path);
        return false;
    }
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CorpusInsertionMode {
    Live,
    MemoryOnly,
}

struct ReplayOutcome {
    keep_entry: bool,
    new_coverage: bool,
    /// Whether replay hit a first-time edge (advances the per-worker "time since new edge" timer).
    new_edge: bool,
    cmp_seq: Vec<Vec<ComparisonHint>>,
    failed_replays: usize,
}

#[derive(Clone, Copy)]
pub struct StatelessReplayTarget<'a> {
    pub function: &'a Function,
    pub address: Address,
}

impl StatelessReplayTarget<'_> {
    fn can_replay(self, tx: &BasicTxDetails) -> bool {
        tx.call_details.target == self.address
            && tx
                .call_details
                .calldata
                .get(..4)
                .is_some_and(|selector| self.function.selector() == selector)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ReplayTarget<'a> {
    pub(crate) stateless: Option<StatelessReplayTarget<'a>>,
    pub(crate) fuzzed_contracts: Option<&'a FuzzRunIdentifiedContracts>,
    pub(crate) dynamic: Option<&'a DynamicTargetCtx<'a>>,
}

struct ReplayCoverage<'a> {
    history_map: &'a mut Vec<u8>,
    edge_indices: &'a mut EdgeIndexMap,
    sancov_history_map: &'a mut Vec<u8>,
    metrics: Option<&'a mut CorpusMetrics>,
}

/// Campaign-level corpus state produced by replaying persisted corpus entries once.
///
/// Parallel invariant workers clone this seed so every worker starts with the same warmed corpus
/// and coverage maps. That avoids each worker rediscovering persisted coverage relative to an empty
/// local map.
#[derive(Clone, Default)]
pub(crate) struct WorkerCorpusSeed {
    in_memory_corpus: Vec<CorpusEntry>,
    history_map: Vec<u8>,
    edge_indices: EdgeIndexMap,
    sancov_history_map: Vec<u8>,
    metrics: CorpusMetrics,
    replay_dirs: Option<Vec<PathBuf>>,
    failed_replays: usize,
    optimization_best_value: Option<I256>,
    optimization_best_sequence: Vec<BasicTxDetails>,
    /// Set if persisted-corpus replay hit a first-time edge, so the timer starts at the baseline
    /// load instead of reading "never" while `cumulative_edges_seen` is non-zero.
    last_new_edge_at: Option<Instant>,
}

impl WorkerCorpusSeed {
    fn empty(config: &FuzzCorpusConfig) -> Self {
        // Hash mode always merges a fixed `MAX_EDGE_COUNT` bitmap, so preallocate to avoid moving
        // the one-time 64 KiB resize into the first merge. Collision-free and sancov maps grow on
        // demand and start empty.
        let history_map =
            if config.collect_evm_edge_coverage() && !config.evm_edge_coverage_collision_free() {
                vec![0u8; MAX_EDGE_COUNT]
            } else {
                Vec::new()
            };
        Self { history_map, ..Default::default() }
    }

    fn with_optimization_state(mut self, config: &FuzzCorpusConfig) -> Self {
        if let Some((value, sequence)) = load_optimization_state(config) {
            self.optimization_best_value = Some(value);
            self.optimization_best_sequence = sequence;
        }
        self
    }

    pub(crate) fn clone_for_worker(
        &self,
        worker_id: usize,
        worker_count: usize,
        include_cmp_seq: bool,
    ) -> Self {
        let in_memory_corpus = self
            .in_memory_corpus
            .iter()
            .enumerate()
            .filter(|(idx, _)| idx % worker_count == worker_id)
            .map(|(_, entry)| {
                let mut entry = entry.clone();
                if !include_cmp_seq {
                    entry.cmp_seq.clear();
                }
                entry
            })
            .collect::<Vec<_>>();

        let mut metrics = self.metrics.clone();
        metrics.corpus_count = in_memory_corpus.len();
        metrics.favored_items = in_memory_corpus.iter().filter(|entry| entry.is_favored).count();

        Self {
            in_memory_corpus,
            history_map: self.history_map.clone(),
            edge_indices: self.edge_indices.clone(),
            sancov_history_map: self.sancov_history_map.clone(),
            metrics,
            replay_dirs: self.replay_dirs.clone(),
            failed_replays: self.failed_replays,
            optimization_best_value: self.optimization_best_value,
            optimization_best_sequence: self.optimization_best_sequence.clone(),
            last_new_edge_at: self.last_new_edge_at,
        }
    }

    pub(crate) fn retain_replayable(&mut self, targeted_contracts: &TargetedContracts) {
        let is_replayable =
            |tx_seq: &[BasicTxDetails]| tx_seq.iter().all(|tx| targeted_contracts.can_replay(tx));
        self.in_memory_corpus.retain(|entry| is_replayable(&entry.tx_seq));
        self.metrics.corpus_count = self.in_memory_corpus.len();
        self.metrics.favored_items =
            self.in_memory_corpus.iter().filter(|entry| entry.is_favored).count();

        if !self.optimization_best_sequence.is_empty()
            && !is_replayable(&self.optimization_best_sequence)
        {
            self.optimization_best_value = None;
            self.optimization_best_sequence.clear();
        }
    }

    pub(crate) fn load_from_disk<FEN: FoundryEvmNetwork>(
        config: &FuzzCorpusConfig,
        replay_root: Option<&Path>,
        executor: Option<&Executor<FEN>>,
        target: ReplayTarget<'_>,
    ) -> Result<Self> {
        let mut seed = Self::empty(config).with_optimization_state(config);
        let Some(corpus_dir) = &config.corpus_dir else {
            return Ok(seed);
        };
        let replay_dirs = canonical_replay_dirs(replay_root.unwrap_or(corpus_dir));
        seed.replay_dirs = Some(replay_dirs.clone());

        // Seed in-memory corpus with the persisted optimization best sequence so the mutation
        // engine can build on it in future runs.
        if !seed.optimization_best_sequence.is_empty() {
            seed.in_memory_corpus.push(CorpusEntry::new(seed.optimization_best_sequence.clone()));
            seed.metrics.corpus_count += 1;
        }

        if target.fuzzed_contracts.is_some() && has_legacy_invariant_corpus_dirs(corpus_dir) {
            let _ = sh_warn!(
                "Ignoring legacy invariant corpus directories under {}; new corpus entries are persisted under the contract-level corpus directory.",
                corpus_dir.display(),
            );
        }

        let Some(executor) = executor else {
            return Ok(seed);
        };
        let mut seen_entries =
            seed.in_memory_corpus.iter().map(|entry| entry.uuid).collect::<HashSet<_>>();
        for entry in unique_corpus_entries(&replay_dirs, &mut seen_entries) {
            // A corrupt or truncated corpus file must not abort the whole campaign startup: skip
            // it and keep loading the rest of the corpus. Canonical entries are atomically
            // published, but malformed files may come from older versions or manual edits.
            let tx_seq = match entry.read_tx_seq() {
                Ok(tx_seq) => tx_seq,
                Err(err) => {
                    let _ =
                        sh_warn!("Skipping unreadable corpus file {}: {err}", entry.path.display());
                    continue;
                }
            };
            if tx_seq.is_empty() {
                continue;
            }

            let coverage = ReplayCoverage {
                history_map: &mut seed.history_map,
                edge_indices: &mut seed.edge_indices,
                sancov_history_map: &mut seed.sancov_history_map,
                metrics: Some(&mut seed.metrics),
            };
            let ReplayOutcome { keep_entry, new_edge, cmp_seq, failed_replays, .. } =
                replay_corpus_sequence(&tx_seq, executor, target, coverage)?;
            seed.failed_replays += failed_replays;
            // Start the timer at the baseline load if replay hit a first-time edge.
            if new_edge {
                seed.last_new_edge_at = Some(Instant::now());
            }
            if !keep_entry {
                continue;
            }

            seed.metrics.corpus_count += 1;
            debug!(
                target: "corpus",
                "load sequence with len {} from corpus file {}",
                tx_seq.len(),
                entry.path.display()
            );
            seed.in_memory_corpus.push(CorpusEntry::new_with_cmp(tx_seq, cmp_seq, entry.uuid));
        }

        Ok(seed)
    }
}

#[derive(Default)]
pub(crate) struct GlobalCorpusMetrics {
    // Number of edges seen during the invariant run.
    cumulative_edges_seen: AtomicUsize,
    // Number of features (new hitcount bin of previously hit edge) seen during the invariant run.
    cumulative_features_seen: AtomicUsize,
    // Number of corpus entries.
    corpus_count: AtomicUsize,
    // Number of corpus entries that are favored.
    favored_items: AtomicUsize,
}

pub(crate) struct CorpusSyncCoordinator {
    workers: usize,
    arrived: AtomicUsize,
    phase: AtomicUsize,
    aborted: AtomicBool,
}

impl CorpusSyncCoordinator {
    pub(crate) const fn new(workers: usize) -> Self {
        Self {
            workers,
            arrived: AtomicUsize::new(0),
            phase: AtomicUsize::new(0),
            aborted: AtomicBool::new(false),
        }
    }

    pub(crate) fn abort(&self) {
        self.aborted.store(true, Ordering::Release);
    }

    fn wait(&self) -> bool {
        if self.aborted.load(Ordering::Acquire) {
            return false;
        }

        let phase = self.phase.load(Ordering::Acquire);
        if self.arrived.fetch_add(1, Ordering::AcqRel) + 1 == self.workers {
            self.arrived.store(0, Ordering::Release);
            self.phase.fetch_add(1, Ordering::AcqRel);
            return true;
        }

        while self.phase.load(Ordering::Acquire) == phase && !self.aborted.load(Ordering::Acquire) {
            rayon::yield_now();
        }
        !self.aborted.load(Ordering::Acquire)
    }
}

impl fmt::Display for GlobalCorpusMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.load().fmt(f)
    }
}

impl GlobalCorpusMetrics {
    pub(crate) fn load(&self) -> CorpusMetrics {
        CorpusMetrics {
            cumulative_edges_seen: self.cumulative_edges_seen.load(Ordering::Relaxed),
            cumulative_features_seen: self.cumulative_features_seen.load(Ordering::Relaxed),
            corpus_count: self.corpus_count.load(Ordering::Relaxed),
            favored_items: self.favored_items.load(Ordering::Relaxed),
        }
    }
}

#[derive(Serialize, Default, Clone)]
pub(crate) struct CorpusMetrics {
    // Number of edges seen during the invariant run.
    cumulative_edges_seen: usize,
    // Number of features (new hitcount bin of previously hit edge) seen during the invariant run.
    cumulative_features_seen: usize,
    // Number of corpus entries.
    corpus_count: usize,
    // Number of corpus entries that are favored.
    favored_items: usize,
}

impl fmt::Display for CorpusMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f)?;
        writeln!(f, "      Edge coverage metrics:")?;
        writeln!(f, "        - cumulative edges seen: {}", self.cumulative_edges_seen)?;
        writeln!(f, "        - cumulative features seen: {}", self.cumulative_features_seen)?;
        writeln!(f, "        - corpus count: {}", self.corpus_count)?;
        write!(f, "        - favored items: {}", self.favored_items)?;
        Ok(())
    }
}

impl CorpusMetrics {
    /// Records number of new edges or features explored during the campaign.
    pub const fn update_seen(&mut self, is_edge: bool) {
        if is_edge {
            self.cumulative_edges_seen += 1;
        } else {
            self.cumulative_features_seen += 1;
        }
    }

    /// Updates campaign favored items.
    pub const fn update_favored(&mut self, is_favored: bool, corpus_favored: bool) {
        if is_favored && !corpus_favored {
            self.favored_items += 1;
        } else if !is_favored && corpus_favored {
            self.favored_items -= 1;
        }
    }
}

/// Per-worker corpus manager.
pub struct WorkerCorpus {
    /// Worker Id
    id: usize,
    /// In-memory corpus entries populated from the persisted files and
    /// runs administered by this worker.
    in_memory_corpus: Vec<CorpusEntry>,
    /// History of binned hitcount of edges seen during fuzzing
    history_map: Vec<u8>,
    /// Stable dense EVM edge IDs for this worker's history map.
    edge_indices: EdgeIndexMap,
    /// History of binned hitcount of sancov (native Rust) edges seen during fuzzing
    sancov_history_map: Vec<u8>,
    /// Number of failed replays from initial corpus
    pub(crate) failed_replays: usize,
    /// Worker Metrics
    pub(crate) metrics: CorpusMetrics,
    /// Shared transaction-sequence generator.
    sequence_generator: SequenceGenerator,
    /// Identifier of current mutated entry for this worker.
    current_mutated_index: Option<usize>,
    /// Config
    config: Arc<FuzzCorpusConfig>,
    /// Whether this corpus participates in stateless worker synchronization.
    worker_sync_enabled: bool,
    /// Sorted indices of new entries added to [`WorkerCorpus::in_memory_corpus`] since last sync.
    new_entry_indices: Vec<usize>,
    /// Corpus directories the master loaded at startup and still needs to distribute.
    initial_export_dirs: Option<Vec<PathBuf>>,
    /// Worker Dir
    /// corpus_dir/worker1/
    worker_dir: Option<PathBuf>,
    /// Whether this worker already warned that a live corpus entry could not be persisted.
    warned_persistence_failure: bool,
    /// Metrics at last sync - used to calculate deltas while syncing with global metrics
    last_sync_metrics: CorpusMetrics,
    /// Optimization mode: the best value found so far (loaded from disk or discovered in-run).
    optimization_best_value: Option<I256>,
    /// Optimization mode: the call sequence that produced the best value.
    optimization_best_sequence: Vec<BasicTxDetails>,
    /// Monotonic time the worker's local map last gained a first-time edge; `None` until then.
    ///
    /// Updated wherever the map grows: live fuzzing, startup replay, and cross-worker sync. Tracks
    /// *local* discovery (an edge new to this worker), not globally unique discovery. Kept out of
    /// [`CorpusMetrics`] since a timestamp is neither additive across workers nor serializable.
    last_new_edge_at: Option<Instant>,
}

/// Refs used during corpus replay to register contracts deployed mid-sequence as fuzz targets,
/// mirroring the campaign loop so follow-up calls into them aren't dropped by `can_replay_tx`.
#[derive(Clone, Copy)]
pub struct DynamicTargetCtx<'a> {
    pub project_contracts: &'a ContractsByArtifact,
    pub setup_contracts: &'a ContractsByAddress,
    pub artifact_filters: &'a ArtifactFilters,
}

/// Registers contracts created by the last tx so subsequent txs in the same replayed sequence
/// can target them.
pub(crate) fn register_replay_created(
    state_changeset: &StateChangeset,
    dynamic: Option<&DynamicTargetCtx<'_>>,
    fuzzed_contracts: Option<&FuzzRunIdentifiedContracts>,
    created: &mut Vec<Address>,
) {
    let (Some(dynamic), Some(fuzzed_contracts)) = (dynamic, fuzzed_contracts) else {
        return;
    };
    if let Err(error) = fuzzed_contracts.collect_created_contracts(
        state_changeset,
        dynamic.project_contracts,
        dynamic.setup_contracts,
        dynamic.artifact_filters,
        created,
    ) {
        warn!(target: "corpus", "{error}");
    }
}

/// Clears dynamic targets added during a replayed entry so they don't leak into the next one.
pub(crate) fn rollback_replay_created(
    fuzzed_contracts: Option<&FuzzRunIdentifiedContracts>,
    created: Vec<Address>,
) {
    if !created.is_empty()
        && let Some(fuzzed_contracts) = fuzzed_contracts
    {
        fuzzed_contracts.clear_created_contracts(created);
    }
}

fn load_optimization_state(config: &FuzzCorpusConfig) -> Option<(I256, Vec<BasicTxDetails>)> {
    let corpus_dir = config.corpus_dir.as_ref()?;
    let opt_path = corpus_dir.join(OPTIMIZATION_BEST_FILE);
    if !opt_path.is_file() {
        return None;
    }

    match foundry_common::fs::read_json_file::<OptimizationState>(&opt_path) {
        Ok(state) => {
            debug!(
                target: "corpus",
                "loaded optimization best value {} with sequence len {}",
                state.best_value,
                state.best_sequence.len()
            );
            Some((state.best_value, state.best_sequence))
        }
        Err(err) => {
            let _ = sh_warn!(
                "failed to load optimization state from {}: {err}; starting without persisted optimization seed",
                opt_path.display()
            );
            None
        }
    }
}

fn replay_corpus_sequence<FEN: FoundryEvmNetwork>(
    tx_seq: &[BasicTxDetails],
    executor: &Executor<FEN>,
    target: ReplayTarget<'_>,
    coverage: ReplayCoverage<'_>,
) -> Result<ReplayOutcome> {
    let mut executor = executor.clone();
    replay_corpus_sequence_with_executor(tx_seq, &mut executor, target, coverage, false, true)
}

fn replay_corpus_sequence_with_executor<FEN: FoundryEvmNetwork>(
    tx_seq: &[BasicTxDetails],
    executor: &mut Executor<FEN>,
    target: ReplayTarget<'_>,
    mut coverage: ReplayCoverage<'_>,
    trace_sync: bool,
    reject_unmatched_function: bool,
) -> Result<ReplayOutcome> {
    let mut cmp_seq = Vec::with_capacity(tx_seq.len());
    let mut failed_replays = 0;
    let mut new_coverage_for_entry = false;
    let mut new_edge_for_entry = false;
    let mut created: Vec<Address> = Vec::new();

    for tx in tx_seq {
        if WorkerCorpus::can_replay_tx(tx, target.stateless, target.fuzzed_contracts) {
            let mut call_result = execute_tx(executor, tx)?;
            cmp_seq.push(
                call_result
                    .evm_cmp_values
                    .take()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|cmp| ComparisonHint { lhs: cmp.op1, rhs: cmp.op2 })
                    .collect(),
            );
            let (new_coverage, is_edge) = call_result.merge_all_coverage(
                coverage.history_map,
                coverage.edge_indices,
                coverage.sancov_history_map,
            );
            if new_coverage {
                new_coverage_for_entry = true;
                new_edge_for_entry |= is_edge;
                if let Some(metrics) = coverage.metrics.as_deref_mut() {
                    metrics.update_seen(is_edge);
                }
            }

            register_replay_created(
                &call_result.state_changeset,
                target.dynamic,
                target.fuzzed_contracts,
                &mut created,
            );

            // Commit only when running invariant / stateful tests.
            if target.fuzzed_contracts.is_some() {
                executor.commit(&mut call_result);
            }

            if trace_sync {
                trace!(
                    target: "corpus",
                    %new_coverage,
                    ?tx,
                    "replayed tx for syncing",
                );
            }
        } else {
            cmp_seq.push(Vec::new());
            failed_replays += 1;

            if reject_unmatched_function && target.stateless.is_some() {
                rollback_replay_created(target.fuzzed_contracts, created);
                return Ok(ReplayOutcome {
                    keep_entry: false,
                    new_coverage: new_coverage_for_entry,
                    new_edge: new_edge_for_entry,
                    cmp_seq,
                    failed_replays,
                });
            }
        }
    }
    rollback_replay_created(target.fuzzed_contracts, created);

    Ok(ReplayOutcome {
        keep_entry: true,
        new_coverage: new_coverage_for_entry,
        new_edge: new_edge_for_entry,
        cmp_seq,
        failed_replays,
    })
}

impl WorkerCorpus {
    /// Produces the next sequence for either execution mode.
    pub fn new_sequence(&mut self, test_runner: &mut TestRunner) -> Result<SequencePlan> {
        if self.config.is_coverage_guided() && !self.in_memory_corpus.is_empty() {
            self.evict_oldest_corpus()?;
        }
        let corpus_len = self.in_memory_corpus.len();
        let plan = self.sequence_generator.start(
            test_runner,
            corpus_len,
            |index| {
                let entry = self
                    .in_memory_corpus
                    .get(index)
                    .ok_or_else(|| eyre::eyre!("corpus index {index} is out of bounds"))?;
                CorpusEntryView::new(&entry.tx_seq, &entry.cmp_seq)
            },
            self.config.is_coverage_guided(),
        )?;
        self.current_mutated_index = plan.source();
        Ok(plan)
    }

    pub fn new<FEN: FoundryEvmNetwork>(
        id: usize,
        config: FuzzCorpusConfig,
        sequence_generator: SequenceGenerator,
        replay_root: Option<&Path>,
        // Only required by master worker (id = 0) to replay existing corpus.
        executor: Option<&Executor<FEN>>,
        target: ReplayTarget<'_>,
    ) -> Result<Self> {
        let seed = if id == 0 {
            WorkerCorpusSeed::load_from_disk(&config, replay_root, executor, target)?
        } else {
            WorkerCorpusSeed::empty(&config).with_optimization_state(&config)
        };
        let mut corpus = Self::from_seed(id, config, sequence_generator, seed)?;
        corpus.worker_sync_enabled = true;
        Ok(corpus)
    }

    pub(crate) fn from_seed(
        id: usize,
        config: FuzzCorpusConfig,
        sequence_generator: SequenceGenerator,
        mut seed: WorkerCorpusSeed,
    ) -> Result<Self> {
        let initial_export_dirs = if id == 0 { seed.replay_dirs.take() } else { None };
        let worker_dir = if let Some(corpus_dir) = &config.corpus_dir {
            let worker_dir = corpus_dir.join(format!("{WORKER}{id}"));
            let worker_corpus = worker_dir.join(CORPUS_DIR);
            let sync_dir = worker_dir.join(SYNC_DIR);

            // Create the necessary directories for the worker.
            foundry_common::fs::create_dir_all(&worker_corpus)?;
            foundry_common::fs::create_dir_all(&sync_dir)?;

            Some(worker_dir)
        } else {
            None
        };

        Ok(Self {
            id,
            in_memory_corpus: seed.in_memory_corpus,
            history_map: seed.history_map,
            edge_indices: seed.edge_indices,
            sancov_history_map: seed.sancov_history_map,
            failed_replays: seed.failed_replays,
            metrics: seed.metrics,
            sequence_generator,
            current_mutated_index: None,
            config: config.into(),
            worker_sync_enabled: false,
            new_entry_indices: Default::default(),
            initial_export_dirs,
            worker_dir,
            warned_persistence_failure: false,
            last_sync_metrics: Default::default(),
            optimization_best_value: seed.optimization_best_value,
            optimization_best_sequence: seed.optimization_best_sequence,
            last_new_edge_at: seed.last_new_edge_at,
        })
    }

    /// Updates stats for the given call sequence, if new coverage produced.
    /// Persists the call sequence (if corpus directory is configured and new coverage or
    /// improved optimization value) and updates in-memory corpus.
    #[instrument(skip_all)]
    pub fn process_inputs(
        &mut self,
        inputs: &[BasicTxDetails],
        cmp_seq: &[Vec<CmpOperands>],
        new_coverage: bool,
        optimization: Option<(I256, Vec<BasicTxDetails>)>,
    ) {
        self.process_inputs_inner(
            inputs,
            cmp_seq,
            new_coverage,
            optimization,
            CorpusInsertionMode::Live,
            true,
        );
    }

    /// Updates worker-local corpus state and persists interesting inputs immediately, while
    /// leaving campaign-wide optimization persistence to the coordinator. Entries are retained
    /// per worker rather than globally filtered so abrupt termination cannot discard discoveries
    /// that have not yet reached the coordinator.
    #[instrument(skip_all)]
    pub fn process_inputs_for_campaign(
        &mut self,
        inputs: &[BasicTxDetails],
        cmp_seq: &[Vec<CmpOperands>],
        new_coverage: bool,
        optimization: Option<(I256, Vec<BasicTxDetails>)>,
    ) {
        self.process_inputs_inner(
            inputs,
            cmp_seq,
            new_coverage,
            optimization,
            CorpusInsertionMode::Live,
            false,
        );
    }

    fn process_inputs_inner(
        &mut self,
        inputs: &[BasicTxDetails],
        cmp_seq: &[Vec<CmpOperands>],
        new_coverage: bool,
        optimization: Option<(I256, Vec<BasicTxDetails>)>,
        insertion_mode: CorpusInsertionMode,
        persist_optimization: bool,
    ) {
        // Check if this run improved the optimization value.
        let improved_optimization = optimization.as_ref().is_some_and(|(value, _)| {
            self.optimization_best_value.is_none_or(|best| *value > best)
        });

        // Update stats of current mutated primary corpus.
        if let Some(index) = self.current_mutated_index.take() {
            let should_credit = new_coverage || improved_optimization;
            if let Some(corpus) = self.in_memory_corpus.get_mut(index) {
                corpus.total_mutations += 1;
                if should_credit {
                    corpus.new_finds_produced += 1
                }
                let is_favored = (corpus.new_finds_produced as f64 / corpus.total_mutations as f64)
                    > FAVORABILITY_THRESHOLD;
                self.metrics.update_favored(is_favored, corpus.is_favored);
                corpus.is_favored = is_favored;

                trace!(
                    target: "corpus",
                    "updated corpus {}, total mutations: {}, new finds: {}",
                    corpus.uuid, corpus.total_mutations, corpus.new_finds_produced
                );
            }
        }
        if let Some((value, best_seq)) = optimization
            && improved_optimization
        {
            self.optimization_best_value = Some(value);
            self.optimization_best_sequence = best_seq;
            if persist_optimization {
                self.persist_optimization_state();
            }
        }

        if !self.config.is_coverage_guided() {
            return;
        }

        // Collect inputs if current run produced new coverage or improved optimization.
        if !new_coverage && !improved_optimization {
            return;
        }

        // When the run is interesting only because of optimization (no new coverage),
        // add the best prefix to the corpus instead of the full run — the prefix is
        // the sequence that actually achieved the best value.
        //
        // `inputs` can be empty when every call was discarded/popped but new coverage was
        // still recorded; there's nothing to persist, so skip without inserting an entry.
        let corpus_inputs = if improved_optimization && (!new_coverage || inputs.is_empty()) {
            self.optimization_best_sequence.clone()
        } else {
            inputs.to_vec()
        };
        if corpus_inputs.is_empty() {
            return;
        }
        let corpus_cmp_seq = cmp_seq
            .iter()
            .take(corpus_inputs.len())
            .map(|values| {
                values.iter().map(|cmp| ComparisonHint { lhs: cmp.op1, rhs: cmp.op2 }).collect()
            })
            .collect();
        let corpus = CorpusEntry::new_with_cmp(corpus_inputs, corpus_cmp_seq, Uuid::new_v4());

        self.insert_corpus_entry(corpus, insertion_mode)
    }

    fn insert_corpus_entry(&mut self, corpus: CorpusEntry, insertion_mode: CorpusInsertionMode) {
        if matches!(insertion_mode, CorpusInsertionMode::Live)
            && let Some(worker_dir) = &self.worker_dir
        {
            let worker_corpus = worker_dir.join(CORPUS_DIR);
            let write_result = corpus.write_to_disk_in(&worker_corpus, self.config.corpus_gzip);
            if let Err(err) = write_result {
                if !self.warned_persistence_failure {
                    let _ = sh_warn!(
                        "Failed to persist coverage corpus entries for worker {} in {}: {err}",
                        self.id,
                        worker_corpus.display()
                    );
                    self.warned_persistence_failure = true;
                }
                debug!(target: "corpus", %err, "failed to record call sequence {:?}", corpus.tx_seq);
            } else {
                trace!(
                    target: "corpus",
                    "persisted {} inputs for new coverage for {} corpus",
                    corpus.tx_seq.len(),
                    corpus.uuid,
                );
            }
        }

        self.push_corpus_entry(corpus);
    }

    fn push_corpus_entry(&mut self, corpus: CorpusEntry) {
        let new_index = self.in_memory_corpus.len();
        if self.worker_sync_enabled {
            self.new_entry_indices.push(new_index);
        }
        self.metrics.corpus_count += 1;
        self.in_memory_corpus.push(corpus);
    }

    /// Returns the previously persisted optimization best value and sequence (if any).
    pub fn optimization_initial_state(&self) -> (Option<I256>, Vec<BasicTxDetails>) {
        (self.optimization_best_value, self.optimization_best_sequence.clone())
    }

    /// Persists the current optimization best value and sequence to disk.
    fn persist_optimization_state(&self) {
        let optimization_best = self
            .optimization_best_value
            .map(|value| (value, self.optimization_best_sequence.as_slice()));
        persist_optimization_output(&self.config, optimization_best);
    }

    /// Collects EVM and sancov coverage from call result and updates metrics.
    pub fn merge_edge_coverage<FEN: FoundryEvmNetwork>(
        &mut self,
        call_result: &mut RawCallResult<FEN>,
    ) -> bool {
        if !self.config.collect_edge_coverage() {
            return false;
        }

        let (new_coverage, is_edge) = call_result.merge_all_coverage(
            &mut self.history_map,
            &mut self.edge_indices,
            &mut self.sancov_history_map,
        );
        if new_coverage {
            self.metrics.update_seen(is_edge);
            // Only a first-time edge (not a new hitcount bucket, i.e. a "feature") resets the
            // timer.
            if is_edge {
                self.last_new_edge_at = Some(Instant::now());
            }
        }
        new_coverage
    }

    /// Time since this worker last gained a first-time edge; `None` until it has seen one. See
    /// [`WorkerCorpus::last_new_edge_at`] for the local-vs-global caveat.
    pub(crate) fn time_since_new_edge(&self) -> Option<Duration> {
        self.last_new_edge_at.map(|at| at.elapsed())
    }
    /// Converts replayable observed sub-calls into one normal multi-transaction corpus entry.
    ///
    /// This captures calls shaped by a handler or another target call and lets the existing corpus
    /// machinery mutate, evict, sync, and persist them like any other interesting sequence.
    pub fn hoist_observed_calls(
        &mut self,
        observed: &[ObservedCall],
        parent_tx: &BasicTxDetails,
        targeted_contracts: &FuzzRunIdentifiedContracts,
        insertion_mode: CorpusInsertionMode,
    ) {
        if !self.config.is_coverage_guided() || observed.is_empty() {
            return;
        }

        let tx_seq = {
            let targets = targeted_contracts.targets();
            sequence_from_observed(
                observed,
                &targets,
                ObservedCallDepth::All,
                Some((parent_tx.warp, parent_tx.roll)),
            )
        };

        self.push_observed_sequence(tx_seq, insertion_mode)
    }

    /// Seeds the corpus from sibling zero-input unit tests by replaying them on a clone of the
    /// post-setUp executor and keeping the direct replayable calls made by each test.
    ///
    /// Returns the number of test-derived corpus entries added.
    pub fn seed_from_test_traces<FEN: FoundryEvmNetwork>(
        &mut self,
        invariant_contract: &InvariantContract<'_>,
        targeted_contracts: &FuzzRunIdentifiedContracts,
        executor: &Executor<FEN>,
    ) -> Result<usize> {
        if !self.config.is_coverage_guided() {
            return Ok(0);
        }

        let mut added = 0;

        for func in invariant_contract.abi.functions() {
            if !func.is_unit_test() {
                continue;
            }
            if invariant_contract
                .invariant_fns
                .iter()
                .any(|(invariant_fn, _)| func.selector() == invariant_fn.selector())
            {
                continue;
            }

            let calldata = match func.abi_encode_input(&[]) {
                Ok(calldata) => Bytes::from(calldata),
                Err(_) => continue,
            };

            let exec = executor.clone();

            let raw = match exec.call_raw(CALLER, invariant_contract.address, calldata, U256::ZERO)
            {
                Ok(raw) => raw,
                Err(_) => continue,
            };
            if raw.reverted {
                continue;
            }

            let observed = raw.observed_calls;
            if observed.is_empty() {
                continue;
            }

            let seq = {
                let targets = targeted_contracts.targets();
                sequence_from_observed(&observed, &targets, ObservedCallDepth::DirectOnly, None)
            };

            let insertion_mode = if self.id == 0 {
                CorpusInsertionMode::Live
            } else {
                CorpusInsertionMode::MemoryOnly
            };
            let len_before = self.in_memory_corpus.len();
            self.push_observed_sequence(seq, insertion_mode);
            if self.in_memory_corpus.len() > len_before {
                debug!(target: "corpus", test = %func.name, "seeded corpus sequence from test trace");
                added += 1;
            }
        }

        Ok(added)
    }

    fn push_observed_sequence(
        &mut self,
        tx_seq: Vec<BasicTxDetails>,
        insertion_mode: CorpusInsertionMode,
    ) {
        if !self.config.is_coverage_guided() || tx_seq.is_empty() {
            return;
        }

        let corpus = CorpusEntry::new(tx_seq);

        self.insert_corpus_entry(corpus, insertion_mode)
    }
    /// Flush the oldest corpus mutated more than configured max mutations unless it is favored
    /// or pending synchronization.
    fn evict_oldest_corpus(&mut self) -> Result<()> {
        if self.in_memory_corpus.len() > self.config.corpus_min_size.max(1)
            && let Some(index) =
                self.in_memory_corpus.iter().enumerate().position(|(index, corpus)| {
                    self.new_entry_indices.binary_search(&index).is_err()
                        && corpus.total_mutations > self.config.corpus_min_mutations
                        && !corpus.is_favored
                })
        {
            let corpus = &self.in_memory_corpus[index];

            trace!(target: "corpus", corpus=%serde_json::to_string(&corpus).unwrap(), "evict corpus");

            // Remove corpus from memory.
            self.in_memory_corpus.remove(index);

            // Adjust the tracked indices.
            self.new_entry_indices.retain_mut(|i| {
                if *i > index {
                    *i -= 1; // Shift indices down.
                    true // Keep this index.
                } else {
                    *i != index // Remove if it's the deleted index, keep otherwise.
                }
            });
        }
        Ok(())
    }
    // Sync Methods.

    /// Imports the new corpus entries from the `sync` directory.
    /// These contain tx sequences which are replayed and used to update the history map.
    fn load_sync_corpus(&self, strict: bool) -> Result<Vec<(CorpusDirEntry, Vec<BasicTxDetails>)>> {
        let Some(worker_dir) = &self.worker_dir else {
            return Ok(vec![]);
        };

        let sync_dir = worker_dir.join(SYNC_DIR);
        if !strict && !sync_dir.is_dir() {
            return Ok(vec![]);
        }

        let mut imports = vec![];
        let entries = if strict {
            read_corpus_dir_strict(&sync_dir)?
        } else {
            read_corpus_dir(&sync_dir).collect()
        };
        for entry in entries {
            // A corrupt or truncated sync file must not abort the whole sync pass: skip it.
            let tx_seq = match entry.read_tx_seq() {
                Ok(tx_seq) => tx_seq,
                Err(err) if strict => {
                    return Err(eyre!(
                        "failed to read final corpus entry {}: {err}",
                        entry.path.display()
                    ));
                }
                Err(err) => {
                    warn!(target: "corpus", "skipping unreadable corpus file {}: {err}", entry.path.display());
                    let quarantine_path = entry.path.with_file_name(format!(
                        "{}.{}.invalid",
                        entry.name(),
                        Uuid::new_v4()
                    ));
                    if let Err(err) = std::fs::rename(&entry.path, &quarantine_path) {
                        debug!(target: "corpus", %err, "failed to quarantine unreadable corpus file {}", entry.path.display());
                    }
                    continue;
                }
            };
            if tx_seq.is_empty() {
                warn!(target: "corpus", "skipping empty corpus entry: {}", entry.path.display());
                if let Err(err) = std::fs::remove_file(&entry.path) {
                    if strict {
                        return Err(err.into());
                    }
                    debug!(target: "corpus", %err, "failed to remove empty corpus file {}", entry.path.display());
                }
                continue;
            }
            imports.push((entry, tx_seq));
        }

        if !imports.is_empty() {
            debug!(target: "corpus", "imported {} new corpus entries", imports.len());
        }

        Ok(imports)
    }

    /// Adds a calibrated sync entry to the local corpus and queues it for fan-out on the master.
    fn push_synced_corpus_entry(
        &mut self,
        mut corpus: CorpusEntry,
        timestamp: u64,
        file_name: String,
    ) {
        corpus.timestamp = timestamp;
        corpus.persisted_file_name = Some(file_name);
        if self.worker_sync_enabled && self.id == 0 {
            self.new_entry_indices.push(self.in_memory_corpus.len());
        }
        self.in_memory_corpus.push(corpus);
    }

    /// Syncs and calibrates the in memory corpus and updates the history_map if new coverage is
    /// found from the corpus findings of other workers.
    #[instrument(skip_all)]
    fn calibrate<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        target: ReplayTarget<'_>,
        strict: bool,
    ) -> Result<()> {
        let Some(worker_dir) = &self.worker_dir else {
            return Ok(());
        };
        let corpus_dir = worker_dir.join(CORPUS_DIR);

        for (entry, tx_seq) in self.load_sync_corpus(strict)? {
            let mut history_map = self.history_map.clone();
            let mut edge_indices = self.edge_indices.clone();
            let mut sancov_history_map = self.sancov_history_map.clone();
            let mut metrics = self.metrics.clone();
            let coverage = ReplayCoverage {
                history_map: &mut history_map,
                edge_indices: &mut edge_indices,
                sancov_history_map: &mut sancov_history_map,
                metrics: Some(&mut metrics),
            };
            let mut replay_executor = executor.clone();
            let ReplayOutcome { keep_entry, new_coverage, new_edge, cmp_seq, .. } =
                replay_corpus_sequence_with_executor(
                    &tx_seq,
                    &mut replay_executor,
                    target,
                    coverage,
                    true,
                    false,
                )?;

            let sync_path = &entry.path;
            if keep_entry && new_coverage {
                // Move file from sync/ to corpus/ directory.
                let corpus_path = corpus_dir.join(sync_path.components().next_back().unwrap());
                if !accept_synced_corpus_file(&entry, &tx_seq, &corpus_path) {
                    if strict {
                        return Err(eyre!("failed to accept final corpus entry {}", entry.name()));
                    }
                    continue;
                }

                self.history_map = history_map;
                self.edge_indices = edge_indices;
                self.sancov_history_map = sancov_history_map;
                self.metrics = metrics;
                // A synced edge is new to this worker's local map, so it advances the timer.
                if new_edge {
                    self.last_new_edge_at = Some(Instant::now());
                }

                debug!(
                    target: "corpus",
                    name=%entry.name(),
                    "moved synced corpus to corpus dir",
                );

                let corpus_entry = CorpusEntry::new_with_cmp(tx_seq.clone(), cmp_seq, entry.uuid);
                self.push_synced_corpus_entry(
                    corpus_entry,
                    entry.timestamp,
                    entry.name().to_owned(),
                );
            } else {
                // Remove the file as it did not generate new coverage.
                if let Err(err) = std::fs::remove_file(&entry.path) {
                    if strict {
                        return Err(err.into());
                    }
                    debug!(target: "corpus", %err, "failed to remove synced corpus from {sync_path:?}");
                    continue;
                }
                trace!(target: "corpus", "removed synced corpus from {sync_path:?}");
            }
        }

        Ok(())
    }

    /// Exports the new corpus entries to the master worker's sync dir.
    #[instrument(skip_all)]
    fn export_to_master(&mut self) -> Result<()> {
        // Master doesn't export (it only receives from others).
        assert_ne!(self.id, 0, "non-master only");

        // Early return if no new entries or corpus dir not configured.
        if self.new_entry_indices.is_empty() || self.worker_dir.is_none() {
            return Ok(());
        }

        let worker_dir = self.worker_dir.as_ref().unwrap();
        let Some(master_sync_dir) = self
            .config
            .corpus_dir
            .as_ref()
            .map(|dir| dir.join(format!("{WORKER}0")).join(SYNC_DIR))
        else {
            return Ok(());
        };

        let mut exported = 0;
        let corpus_dir = worker_dir.join(CORPUS_DIR);
        let mut delivered = HashSet::new();

        for &index in &self.new_entry_indices {
            let Some(corpus) = self.in_memory_corpus.get(index) else {
                delivered.insert(index);
                continue;
            };
            let file_name = corpus.file_name(corpus.should_gzip(self.config.corpus_gzip));
            let file_path = corpus_dir.join(&file_name);
            if !file_path.is_file()
                && let Err(err) = corpus.write_to_disk_in(&corpus_dir, self.config.corpus_gzip)
            {
                debug!(target: "corpus", %err, "failed to persist corpus {} for export", corpus.uuid);
                continue;
            }
            let sync_path = master_sync_dir.join(&file_name);
            if !link_corpus_file(&file_path, &sync_path) {
                continue;
            }
            exported += 1;
            delivered.insert(index);
        }
        self.new_entry_indices.retain(|index| !delivered.contains(index));

        debug!(target: "corpus", "exported {exported} new corpus entries");

        Ok(())
    }

    /// Exports the global corpus to the `sync/` directories of all the non-master workers.
    #[instrument(skip_all)]
    fn export_to_workers(&mut self, num_workers: usize) -> Result<()> {
        assert_eq!(self.id, 0, "master worker only");
        if self.worker_dir.is_none() {
            return Ok(());
        }

        let worker_dir = self.worker_dir.as_ref().unwrap();
        let master_corpus_dir = worker_dir.join(CORPUS_DIR);
        let startup_entries = if let Some(replay_dirs) = &self.initial_export_dirs {
            let mut seen_entries = HashSet::new();
            unique_corpus_entries(replay_dirs, &mut seen_entries)
                .map(|entry| entry.path)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let mut pending_entries = Vec::new();
        let mut delivered = HashSet::new();
        for &index in &self.new_entry_indices {
            let Some(corpus) = self.in_memory_corpus.get(index) else {
                delivered.insert(index);
                continue;
            };
            let path = master_corpus_dir
                .join(corpus.file_name(corpus.should_gzip(self.config.corpus_gzip)));
            if !path.is_file()
                && let Err(err) =
                    corpus.write_to_disk_in(&master_corpus_dir, self.config.corpus_gzip)
            {
                debug!(target: "corpus", %err, "failed to persist corpus {} for fan-out", corpus.uuid);
                continue;
            }
            pending_entries.push((index, path));
        }

        let mut target_dirs = Vec::new();
        for target_worker in 1..num_workers {
            let target_dir = self
                .config
                .corpus_dir
                .as_ref()
                .unwrap()
                .join(format!("{WORKER}{target_worker}"))
                .join(SYNC_DIR);
            if !target_dir.is_dir() {
                foundry_common::fs::create_dir_all(&target_dir)?;
            }
            target_dirs.push(target_dir);
        }

        let mut any_distributed = false;
        let mut startup_delivered = true;
        for path in &startup_entries {
            let Some(name) = path.file_name() else {
                startup_delivered = false;
                continue;
            };
            let mut delivered_to_all = true;
            for target_dir in &target_dirs {
                let sync_path = target_dir.join(name);
                if link_corpus_file(path, &sync_path) {
                    any_distributed = true;
                    trace!(target: "corpus", name=%name.to_string_lossy(), ?target_dir, "distributed corpus");
                } else {
                    delivered_to_all = false;
                }
            }
            startup_delivered &= delivered_to_all;
        }

        for (index, path) in pending_entries {
            let Some(name) = path.file_name() else { continue };
            let mut delivered_to_all = true;
            for target_dir in &target_dirs {
                let sync_path = target_dir.join(name);
                if link_corpus_file(&path, &sync_path) {
                    any_distributed = true;
                    trace!(target: "corpus", name=%name.to_string_lossy(), ?target_dir, "distributed corpus");
                } else {
                    delivered_to_all = false;
                }
            }
            if delivered_to_all {
                delivered.insert(index);
            }
        }

        self.new_entry_indices.retain(|index| !delivered.contains(index));
        if startup_delivered {
            self.initial_export_dirs = None;
        }
        debug!(target: "corpus", %any_distributed, "distributed master corpus to all workers");

        Ok(())
    }

    // TODO(dani): currently only master syncs metrics?
    /// Syncs local metrics with global corpus metrics by calculating and applying deltas.
    pub(crate) fn sync_metrics(&mut self, global_corpus_metrics: &GlobalCorpusMetrics) {
        // Calculate delta metrics since last sync.
        let edges_delta = self
            .metrics
            .cumulative_edges_seen
            .saturating_sub(self.last_sync_metrics.cumulative_edges_seen);
        let features_delta = self
            .metrics
            .cumulative_features_seen
            .saturating_sub(self.last_sync_metrics.cumulative_features_seen);
        // For corpus count and favored items, calculate deltas.
        let corpus_count_delta =
            self.metrics.corpus_count as isize - self.last_sync_metrics.corpus_count as isize;
        let favored_delta =
            self.metrics.favored_items as isize - self.last_sync_metrics.favored_items as isize;

        // Add delta values to global metrics.

        if edges_delta > 0 {
            global_corpus_metrics.cumulative_edges_seen.fetch_add(edges_delta, Ordering::Relaxed);
        }
        if features_delta > 0 {
            global_corpus_metrics
                .cumulative_features_seen
                .fetch_add(features_delta, Ordering::Relaxed);
        }

        if corpus_count_delta > 0 {
            global_corpus_metrics
                .corpus_count
                .fetch_add(corpus_count_delta as usize, Ordering::Relaxed);
        } else if corpus_count_delta < 0 {
            global_corpus_metrics
                .corpus_count
                .fetch_sub((-corpus_count_delta) as usize, Ordering::Relaxed);
        }

        if favored_delta > 0 {
            global_corpus_metrics
                .favored_items
                .fetch_add(favored_delta as usize, Ordering::Relaxed);
        } else if favored_delta < 0 {
            global_corpus_metrics
                .favored_items
                .fetch_sub((-favored_delta) as usize, Ordering::Relaxed);
        }

        // Store current metrics as last sync metrics for next delta calculation.
        self.last_sync_metrics = self.metrics.clone();
    }

    /// Syncs the workers in_memory_corpus and history_map with the findings from other workers.
    #[instrument(skip_all)]
    pub fn sync<FEN: FoundryEvmNetwork>(
        &mut self,
        num_workers: usize,
        executor: &Executor<FEN>,
        target: ReplayTarget<'_>,
        global_corpus_metrics: &GlobalCorpusMetrics,
    ) -> Result<()> {
        trace!(target: "corpus", "syncing");

        self.sync_metrics(global_corpus_metrics);

        self.calibrate(executor, target, false)?;
        if self.id == 0 {
            self.export_to_workers(num_workers)?;
        } else {
            self.export_to_master()?;
        }

        debug!(target: "corpus", "synced");

        Ok(())
    }

    /// Performs the ordered final synchronization once every worker has stopped fuzzing.
    pub(crate) fn finalize_sync<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        target: ReplayTarget<'_>,
        coordinator: &CorpusSyncCoordinator,
    ) -> Result<()> {
        if self.id != 0 {
            self.export_to_master()?;
            if self.worker_dir.is_some() && !self.new_entry_indices.is_empty() {
                return Err(eyre!("worker {} failed to complete final corpus export", self.id));
            }
        }
        if !coordinator.wait() {
            return Ok(());
        }

        if self.id == 0 {
            self.calibrate(executor, target, true)?;
            self.export_to_workers(coordinator.workers)?;
            if self.worker_dir.is_some()
                && (!self.new_entry_indices.is_empty() || self.initial_export_dirs.is_some())
            {
                return Err(eyre!("master failed to complete final corpus fan-out"));
            }
        }
        if !coordinator.wait() {
            return Ok(());
        }

        if self.id != 0 {
            self.calibrate(executor, target, true)?;
        }
        if !coordinator.wait() {
            return Ok(());
        }

        Ok(())
    }

    /// Helper to check if a tx can be replayed.
    pub(crate) fn can_replay_tx(
        tx: &BasicTxDetails,
        stateless: Option<StatelessReplayTarget<'_>>,
        fuzzed_contracts: Option<&FuzzRunIdentifiedContracts>,
    ) -> bool {
        fuzzed_contracts.is_some_and(|contracts| contracts.targets().can_replay(tx))
            || stateless.is_some_and(|target| target.can_replay(tx))
    }
}

#[derive(Clone, Copy)]
enum ObservedCallDepth {
    DirectOnly,
    All,
}

fn sequence_from_observed(
    observed: &[ObservedCall],
    targets: &TargetedContracts,
    depth: ObservedCallDepth,
    first_delay: Option<(Option<U256>, Option<U256>)>,
) -> Vec<BasicTxDetails> {
    let mut first_delay = first_delay;
    observed
        .iter()
        .filter(|call| matches!(depth, ObservedCallDepth::All) || call.depth == 1)
        .filter_map(|call| {
            let mut tx = BasicTxDetails {
                warp: None,
                roll: None,
                sender: call.caller,
                call_details: CallDetails {
                    target: call.target,
                    calldata: call.calldata.clone(),
                    value: call.value,
                },
            };
            targets.can_replay(&tx).then(|| {
                let (warp, roll) = first_delay.take().unwrap_or((None, None));
                tx.warp = warp;
                tx.roll = roll;
                tx
            })
        })
        .collect()
}

fn persist_optimization_output(
    config: &FuzzCorpusConfig,
    optimization_best: Option<(I256, &[BasicTxDetails])>,
) {
    let Some(root) = &config.corpus_dir else {
        return;
    };
    let Some((value, sequence)) = optimization_best else {
        return;
    };
    let state = OptimizationState { best_value: value, best_sequence: sequence.to_vec() };
    let path = root.join(OPTIMIZATION_BEST_FILE);
    if let Err(err) = foundry_common::fs::write_json_file(&path, &state) {
        debug!(target: "corpus", %err, "failed to persist optimization state");
    } else {
        trace!(
            target: "corpus",
            "persisted optimization best value {} with sequence len {}",
            value,
            sequence.len()
        );
    }
}

pub(crate) fn persist_campaign_optimization(
    config: &FuzzCorpusConfig,
    value: Option<I256>,
    sequence: &[BasicTxDetails],
) {
    persist_optimization_output(config, value.map(|value| (value, sequence)));
}

fn has_legacy_invariant_corpus_dirs(path: &Path) -> bool {
    std::fs::read_dir(path).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            let path = entry.path();
            path.is_dir()
                && entry.file_name().to_str().is_some_and(|name| !name.starts_with(WORKER))
                && !path.join(OPTIMIZATION_BEST_FILE).is_file()
        })
    })
}

fn unique_corpus_entries<'a>(
    replay_dirs: &'a [PathBuf],
    seen_entries: &'a mut HashSet<Uuid>,
) -> impl Iterator<Item = CorpusDirEntry> + 'a {
    replay_dirs.iter().flat_map(|replay_dir| read_corpus_dir(replay_dir)).filter(|entry| {
        let is_new = seen_entries.insert(entry.uuid);
        if !is_new {
            trace!(target: "corpus", "skipping duplicate corpus entry {}", entry.uuid);
        }
        is_new
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        executors::ExecutorBuilder,
        inspectors::{EdgeCovHit, EdgeCoverage, EdgeKey},
    };
    use alloy_dyn_abi::DynSolValue;
    use foundry_config::FuzzDictionaryConfig;
    use foundry_evm_core::{
        backend::Backend,
        evm::{EthEvmNetwork, EvmEnvFor, TxEnvFor},
    };
    use proptest::prelude::Just;
    use rayon::prelude::*;
    use revm::{
        bytecode::Bytecode,
        database::{CacheDB, EmptyDB},
    };
    use std::fs;

    fn basic_tx() -> BasicTxDetails {
        BasicTxDetails {
            warp: None,
            roll: None,
            sender: Address::ZERO,
            call_details: foundry_evm_fuzz::CallDetails {
                target: Address::ZERO,
                calldata: Bytes::new(),
                value: None,
            },
        }
    }

    fn basic_tx_with_calldata(calldata: impl Into<Bytes>) -> BasicTxDetails {
        let mut tx = basic_tx();
        tx.call_details.calldata = calldata.into();
        tx
    }

    fn tx_for_function(
        target: Address,
        function: &Function,
        args: &[DynSolValue],
    ) -> BasicTxDetails {
        BasicTxDetails {
            warp: None,
            roll: None,
            sender: Address::ZERO,
            call_details: foundry_evm_fuzz::CallDetails {
                target,
                calldata: Bytes::from(function.abi_encode_input(args).unwrap()),
                value: None,
            },
        }
    }

    fn empty_fuzz_state() -> EvmFuzzState {
        EvmFuzzState::new(
            &[],
            &CacheDB::<EmptyDB>::default(),
            FuzzDictionaryConfig::default(),
            None,
        )
    }

    fn temp_corpus_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("foundry-corpus-tests-{}", Uuid::new_v4()));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn corpus_config(corpus_dir: PathBuf) -> FuzzCorpusConfig {
        FuzzCorpusConfig {
            corpus_dir: Some(corpus_dir),
            corpus_gzip: false,
            corpus_min_mutations: 0,
            corpus_min_size: 0,
            ..Default::default()
        }
    }

    fn test_sequence(config: &FuzzCorpusConfig, tx: TxGenerator) -> SequenceGenerator {
        SequenceGenerator::stateless(
            tx,
            empty_fuzz_state().stateless_worker(),
            Function::parse("test(uint256)").unwrap(),
            config,
        )
        .unwrap()
    }

    fn worker_corpus(id: usize, corpus_root: PathBuf, seed: WorkerCorpusSeed) -> WorkerCorpus {
        let config = corpus_config(corpus_root);
        let generator =
            test_sequence(&config, TxGenerator::from_strategy(Just(basic_tx()).boxed()));
        let mut corpus = WorkerCorpus::from_seed(id, config, generator, seed).unwrap();
        corpus.worker_sync_enabled = true;
        corpus
    }

    fn empty_worker_corpus(id: usize, corpus_root: PathBuf) -> WorkerCorpus {
        worker_corpus(id, corpus_root, WorkerCorpusSeed::default())
    }

    #[test]
    fn worker_initialization_fails_when_corpus_directories_cannot_be_created() {
        let corpus_root = temp_corpus_dir().join("not-a-directory");
        fs::write(&corpus_root, b"blocked").unwrap();
        let config = corpus_config(corpus_root);
        let generator =
            test_sequence(&config, TxGenerator::from_strategy(Just(basic_tx()).boxed()));

        assert!(
            WorkerCorpus::from_seed(0, config, generator, WorkerCorpusSeed::default()).is_err()
        );
    }

    fn sync_test_executor(corpus_root: PathBuf, target: Address) -> Executor<EthEvmNetwork> {
        let mut executor = ExecutorBuilder::<EthEvmNetwork>::default().gas_limit(1 << 24).build(
            EvmEnvFor::<EthEvmNetwork>::default(),
            TxEnvFor::<EthEvmNetwork>::default(),
            Backend::spawn(None).unwrap(),
            Default::default(),
        );
        executor.inspector_mut().collect_edge_coverage_with_config(&corpus_config(corpus_root));
        // CALLDATALOAD(4); PUSH1 8; JUMPI; STOP; JUMPDEST; STOP.
        executor
            .set_code(
                target,
                Bytecode::new_raw(Bytes::from_static(&[
                    0x60, 0x04, 0x35, 0x60, 0x08, 0x57, 0x00, 0x00, 0x5b, 0x00,
                ])),
            )
            .unwrap();
        executor
    }

    fn finalize_test_worker(
        worker: &mut WorkerCorpus,
        corpus_root: PathBuf,
        target_address: Address,
        coordinator: &CorpusSyncCoordinator,
    ) {
        let function = Function::parse("test(uint256)").unwrap();
        let executor = sync_test_executor(corpus_root, target_address);
        worker
            .finalize_sync(
                &executor,
                ReplayTarget {
                    stateless: Some(StatelessReplayTarget {
                        function: &function,
                        address: target_address,
                    }),
                    fuzzed_contracts: None,
                    dynamic: None,
                },
                coordinator,
            )
            .unwrap();
    }

    fn seeded_worker_corpus(
        id: usize,
        corpus_root: PathBuf,
        entries: Vec<CorpusEntry>,
    ) -> WorkerCorpus {
        worker_corpus(
            id,
            corpus_root,
            WorkerCorpusSeed { in_memory_corpus: entries, ..Default::default() },
        )
    }
    fn new_manager_with_single_corpus() -> (WorkerCorpus, Uuid) {
        let corpus = CorpusEntry::new(vec![basic_tx()]);
        let seed_uuid = corpus.uuid;
        let mut manager = seeded_worker_corpus(0, temp_corpus_dir(), vec![corpus]);
        manager.current_mutated_index = Some(0);

        (manager, seed_uuid)
    }

    fn targeted_contracts_with_selective_functions(
        target: Address,
        functions: Vec<Function>,
        targeted_selectors: impl IntoIterator<Item = alloy_primitives::Selector>,
    ) -> FuzzRunIdentifiedContracts {
        use alloy_json_abi::JsonAbi;
        use foundry_evm_fuzz::invariant::TargetedContract;

        let mut abi = JsonAbi::new();
        for function in functions {
            abi.functions.entry(function.name.clone()).or_default().push(function);
        }

        let mut contract = TargetedContract::new("Target".to_string(), abi);
        contract.add_selectors(targeted_selectors, false).unwrap();

        let mut targets = TargetedContracts::new();
        targets.inner.insert(target, contract);
        FuzzRunIdentifiedContracts::new(targets, false)
    }

    // A corrupt/truncated corpus file (valid name, unparsable content) must surface as a per-entry
    // read error rather than break directory scanning, so the load/sync loops can skip malformed
    // files from older versions or manual edits instead of aborting the whole campaign.
    #[test]
    fn corrupt_corpus_file_surfaces_as_error_for_load_to_skip() {
        let dir = temp_corpus_dir();

        // A valid entry round-trips through the on-disk format.
        let valid = CorpusEntry::new(vec![basic_tx()]);
        valid.write_to_disk_in(&dir, false).unwrap();

        // A file with a valid corpus name but garbage content.
        let corrupt_path = dir.join(format!("{}-123.json", Uuid::new_v4()));
        fs::write(&corrupt_path, b"{ not valid json").unwrap();

        let entries = read_corpus_dir(&dir).collect::<Vec<_>>();
        assert_eq!(entries.len(), 2, "directory scan should surface both files");

        let (mut ok, mut err) = (0u32, 0u32);
        for entry in &entries {
            match entry.read_tx_seq() {
                Ok(seq) => {
                    ok += 1;
                    assert_eq!(seq.len(), 1);
                }
                Err(_) => err += 1,
            }
        }
        assert_eq!((ok, err), (1, 1), "the corrupt file must read as Err, the valid one as Ok");
    }

    #[test]
    fn sync_inbox_loads_entries_regardless_of_timestamp() {
        let corpus_root = temp_corpus_dir();
        let manager = empty_worker_corpus(0, corpus_root.clone());
        let sync_dir = corpus_root.join("worker0").join(SYNC_DIR);
        let mut corpus = CorpusEntry::new(vec![basic_tx()]);
        corpus.timestamp = 0;
        corpus.write_to_disk_in(&sync_dir, false).unwrap();
        let mut empty = CorpusEntry::new(vec![]);
        empty.timestamp = 0;
        let empty_path = empty.write_to_disk_in(&sync_dir, false).unwrap();
        let corrupt_path = sync_dir.join(format!("{}-0.json", Uuid::new_v4()));
        fs::write(&corrupt_path, b"{ not valid json").unwrap();

        let imports = manager.load_sync_corpus(false).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].0.timestamp, 0);
        assert!(!empty_path.exists());
        assert!(!corrupt_path.exists());
        assert!(fs::read_dir(&sync_dir).unwrap().flatten().any(|entry| {
            entry.file_name().to_string_lossy().ends_with(".invalid") && entry.path().is_file()
        }));
    }

    #[test]
    fn synced_entries_are_queued_for_fanout_only_on_master() {
        let corpus_root = temp_corpus_dir();
        let mut master = empty_worker_corpus(0, corpus_root.clone());
        let mut worker = empty_worker_corpus(1, corpus_root);
        let master_entry = CorpusEntry::new(vec![basic_tx()]);
        let worker_entry = CorpusEntry::new(vec![basic_tx()]);

        master.push_synced_corpus_entry(master_entry, 1, "master-1.json".to_string());
        worker.push_synced_corpus_entry(worker_entry, 2, "worker-2.json".to_string());

        assert_eq!(master.new_entry_indices, [0]);
        assert!(worker.new_entry_indices.is_empty());
        assert_eq!(master.in_memory_corpus[0].timestamp, 1);
        assert_eq!(worker.in_memory_corpus[0].timestamp, 2);
        assert_eq!(master.metrics.corpus_count, 0);
        assert_eq!(worker.metrics.corpus_count, 0);
    }

    #[test]
    fn master_distributes_old_synced_entries() {
        let corpus_root = temp_corpus_dir();
        let mut master = empty_worker_corpus(0, corpus_root.clone());
        master.initial_export_dirs = None;
        let mut corpus =
            CorpusEntry::new(vec![basic_tx_with_calldata(vec![0; GZIP_THRESHOLD * 2])]);
        corpus.timestamp = 0;
        let path =
            corpus.write_to_disk_in(&corpus_root.join("worker0").join(CORPUS_DIR), true).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        assert!(name.ends_with(".json.gz"));
        master.push_synced_corpus_entry(corpus, 0, name.clone());

        master.export_to_workers(3).unwrap();

        assert!(corpus_root.join("worker1").join(SYNC_DIR).join(&name).is_file());
        assert!(corpus_root.join("worker2").join(SYNC_DIR).join(name).is_file());
        assert!(master.new_entry_indices.is_empty());
    }

    #[test]
    fn worker_retries_failed_export() {
        let corpus_root = temp_corpus_dir();
        let mut worker = empty_worker_corpus(1, corpus_root.clone());
        let corpus = CorpusEntry::new(vec![basic_tx_with_calldata([1])]);
        let name = corpus.file_name(false);
        worker.push_corpus_entry(corpus);

        let master_sync = corpus_root.join("worker0").join(SYNC_DIR);
        fs::create_dir_all(&master_sync).unwrap();
        let destination = master_sync.join(&name);
        foundry_common::fs::write_json_file(&destination, &vec![basic_tx_with_calldata([2])])
            .unwrap();

        worker.export_to_master().unwrap();
        assert_eq!(worker.new_entry_indices, [0]);
        assert!(corpus_root.join("worker1").join(CORPUS_DIR).join(&name).is_file());

        fs::remove_file(&destination).unwrap();
        worker.export_to_master().unwrap();

        assert!(worker.new_entry_indices.is_empty());
        let exported = read_corpus_dir(&master_sync).next().unwrap().read_tx_seq().unwrap();
        assert!(same_tx_sequence(&exported, &[basic_tx_with_calldata([1])]));
    }

    #[test]
    fn master_retries_partial_fanout() {
        let corpus_root = temp_corpus_dir();
        let mut master = empty_worker_corpus(0, corpus_root.clone());
        master.initial_export_dirs = None;
        let corpus = CorpusEntry::new(vec![basic_tx_with_calldata([1])]);
        let name = corpus.file_name(false);
        corpus.write_to_disk_in(&corpus_root.join("worker0").join(CORPUS_DIR), false).unwrap();
        master.push_corpus_entry(corpus);

        let worker2_sync = corpus_root.join("worker2").join(SYNC_DIR);
        fs::create_dir_all(&worker2_sync).unwrap();
        let worker2_destination = worker2_sync.join(&name);
        foundry_common::fs::write_json_file(
            &worker2_destination,
            &vec![basic_tx_with_calldata([2])],
        )
        .unwrap();

        master.export_to_workers(3).unwrap();
        assert_eq!(master.new_entry_indices, [0]);
        assert!(corpus_root.join("worker1").join(SYNC_DIR).join(&name).is_file());

        fs::remove_file(&worker2_destination).unwrap();
        master.export_to_workers(3).unwrap();

        assert!(master.new_entry_indices.is_empty());
        assert!(worker2_destination.is_file());
    }

    #[test]
    fn final_sync_coordinator_yields_to_nested_rayon_workers() {
        rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap().install(|| {
            (0..2usize).into_par_iter().for_each(|_| {
                let coordinator = CorpusSyncCoordinator::new(2);
                (0..2usize).into_par_iter().for_each(|_| assert!(coordinator.wait()));
            });
        });
    }

    #[test]
    fn final_calibration_rejects_non_file_corpus_entry() {
        let corpus_root = temp_corpus_dir();
        let mut worker = empty_worker_corpus(1, corpus_root.clone());
        let sync_dir = corpus_root.join("worker1").join(SYNC_DIR);
        fs::create_dir(sync_dir.join("00000000-0000-0000-0000-000000000001-1.json")).unwrap();
        let target = Address::repeat_byte(0x11);
        let executor = sync_test_executor(corpus_root, target);
        let function = Function::parse("test(uint256)").unwrap();

        let err = worker
            .calibrate(
                &executor,
                ReplayTarget {
                    stateless: Some(StatelessReplayTarget { function: &function, address: target }),
                    fuzzed_contracts: None,
                    dynamic: None,
                },
                true,
            )
            .unwrap_err()
            .to_string();

        assert!(err.contains("not a regular file"), "{err}");
    }

    #[test]
    fn final_sync_completes_interleaved_corpus_lifecycle() {
        let corpus_root = temp_corpus_dir();
        let function = Function::parse("test(uint256)").unwrap();
        let target_address = Address::repeat_byte(0x11);
        let startup = CorpusEntry::new(vec![tx_for_function(
            target_address,
            &function,
            &[DynSolValue::Uint(U256::ZERO, 256)],
        )]);
        let startup_name = startup.file_name(false);
        fs::create_dir_all(corpus_root.join("worker1").join(CORPUS_DIR)).unwrap();
        startup.write_to_disk_in(&corpus_root.join("worker1").join(CORPUS_DIR), false).unwrap();
        let coordinator = Arc::new(CorpusSyncCoordinator::new(3));
        let setup = Arc::new(std::sync::Barrier::new(3));

        std::thread::scope(|scope| {
            let root = corpus_root.clone();
            let master_startup_name = startup_name.clone();
            let master_coordinator = coordinator.clone();
            let master_setup = setup.clone();
            scope.spawn(move || {
                let function = Function::parse("test(uint256)").unwrap();
                let seed = WorkerCorpusSeed {
                    replay_dirs: Some(canonical_replay_dirs(&root)),
                    ..Default::default()
                };
                let mut master = worker_corpus(0, root.clone(), seed);
                let periodic = CorpusEntry::new(vec![tx_for_function(
                    target_address,
                    &function,
                    &[DynSolValue::Uint(U256::ZERO, 256)],
                )]);
                let periodic_name = periodic.file_name(false);
                periodic.write_to_disk_in(&root.join("worker0").join(CORPUS_DIR), false).unwrap();
                master.push_corpus_entry(periodic);

                // The periodic fan-out reaches worker 1 but remains pending for worker 2.
                let blocked = root.join("worker2").join(SYNC_DIR).join(&periodic_name);
                fs::create_dir_all(&blocked).unwrap();
                master.export_to_workers(3).unwrap();
                assert_eq!(master.new_entry_indices, [0]);
                assert!(root.join("worker2").join(SYNC_DIR).join(master_startup_name).is_file());
                fs::remove_dir(blocked).unwrap();
                master_setup.wait();

                finalize_test_worker(&mut master, root, target_address, &master_coordinator);
            });

            for id in 1..=2 {
                let root = corpus_root.clone();
                let coordinator = coordinator.clone();
                let setup = setup.clone();
                scope.spawn(move || {
                    let mut worker = empty_worker_corpus(id, root.clone());
                    setup.wait();

                    if id == 1 {
                        // This old-timestamp finding appears after the last periodic sync.
                        let function = Function::parse("test(uint256)").unwrap();
                        let mut late = CorpusEntry::new(vec![tx_for_function(
                            target_address,
                            &function,
                            &[DynSolValue::Uint(U256::from(1), 256)],
                        )]);
                        late.timestamp = 0;
                        worker.push_corpus_entry(late);
                    }

                    finalize_test_worker(&mut worker, root, target_address, &coordinator);
                });
            }
        });

        assert_eq!(read_corpus_dir(&corpus_root.join("worker2").join(CORPUS_DIR)).count(), 2);
        assert!(read_corpus_dir(&corpus_root.join("worker2").join(SYNC_DIR)).next().is_none());
    }

    #[test]
    fn master_distributes_startup_corpus_only_once() {
        let corpus_root = temp_corpus_dir();
        let corpus = CorpusEntry::new(vec![basic_tx()]);
        let name = corpus.file_name(false);
        let non_master_corpus = corpus_root.join("worker1").join(CORPUS_DIR);
        fs::create_dir_all(&non_master_corpus).unwrap();
        corpus.write_to_disk_in(&non_master_corpus, false).unwrap();
        let seed = WorkerCorpusSeed {
            replay_dirs: Some(canonical_replay_dirs(&corpus_root)),
            ..Default::default()
        };
        let mut master = worker_corpus(0, corpus_root.clone(), seed);

        let other_worker_sync_dir = corpus_root.join("worker2").join(SYNC_DIR);
        fs::create_dir_all(&other_worker_sync_dir).unwrap();
        let other_worker_sync = other_worker_sync_dir.join(&name);
        foundry_common::fs::write_json_file(&other_worker_sync, &vec![basic_tx_with_calldata([1])])
            .unwrap();

        master.export_to_workers(3).unwrap();
        let source_worker_sync = corpus_root.join("worker1").join(SYNC_DIR).join(&name);
        assert!(source_worker_sync.is_file());
        assert!(other_worker_sync.is_file());
        assert!(master.initial_export_dirs.is_some());

        fs::remove_file(&other_worker_sync).unwrap();
        master.export_to_workers(3).unwrap();
        assert!(other_worker_sync.is_file());
        assert!(master.initial_export_dirs.is_none());

        let source_entry = read_corpus_dir(source_worker_sync.parent().unwrap()).next().unwrap();
        assert!(accept_synced_corpus_file(
            &source_entry,
            &source_entry.read_tx_seq().unwrap(),
            &non_master_corpus.join(&name),
        ));
        assert!(!source_worker_sync.exists());
        assert!(non_master_corpus.join(&name).is_file());

        fs::remove_file(&other_worker_sync).unwrap();
        master.export_to_workers(3).unwrap();
        assert!(!other_worker_sync.exists());

        let flat_root = temp_corpus_dir();
        let flat_corpus = CorpusEntry::new(vec![basic_tx()]);
        let flat_name = flat_corpus.file_name(false);
        flat_corpus.write_to_disk_in(&flat_root, false).unwrap();
        let config = corpus_config(flat_root.clone());
        let seed = WorkerCorpusSeed::load_from_disk::<foundry_evm_core::evm::EthEvmNetwork>(
            &config,
            None,
            None,
            ReplayTarget { stateless: None, fuzzed_contracts: None, dynamic: None },
        )
        .unwrap();
        fs::create_dir_all(flat_root.join("worker1").join(CORPUS_DIR)).unwrap();
        let generator =
            test_sequence(&config, TxGenerator::from_strategy(Just(basic_tx()).boxed()));
        let mut flat_master = WorkerCorpus::from_seed(0, config, generator, seed).unwrap();
        flat_master.worker_sync_enabled = true;
        let pending = CorpusEntry::new(vec![basic_tx_with_calldata([1])]);
        let pending_name = pending.file_name(false);
        pending.write_to_disk_in(&flat_root.join("worker0").join(CORPUS_DIR), false).unwrap();
        flat_master.push_corpus_entry(pending);

        flat_master.export_to_workers(2).unwrap();

        assert!(flat_root.join("worker1").join(SYNC_DIR).join(flat_name).is_file());
        assert!(flat_root.join("worker1").join(SYNC_DIR).join(pending_name).is_file());
        assert!(flat_master.new_entry_indices.is_empty());
    }

    #[test]
    fn pending_master_fanout_entries_are_not_evicted() {
        let corpus_root = temp_corpus_dir();
        let mut master = empty_worker_corpus(0, corpus_root.clone());
        master.initial_export_dirs = None;
        let mut pending = CorpusEntry::new(vec![basic_tx_with_calldata([1])]);
        pending.total_mutations = 1;
        let retained = CorpusEntry::new(vec![basic_tx_with_calldata([2])]);
        let pending_name = pending.file_name(false);
        let retained_name = retained.file_name(false);
        let pending_timestamp = pending.timestamp;
        let retained_timestamp = retained.timestamp;
        let master_corpus = corpus_root.join("worker0").join(CORPUS_DIR);
        pending.write_to_disk_in(&master_corpus, false).unwrap();
        retained.write_to_disk_in(&master_corpus, false).unwrap();
        master.push_synced_corpus_entry(pending, pending_timestamp, pending_name.clone());
        master.push_synced_corpus_entry(retained, retained_timestamp, retained_name.clone());

        master.evict_oldest_corpus().unwrap();
        master.export_to_workers(2).unwrap();

        let worker_sync = corpus_root.join("worker1").join(SYNC_DIR);
        assert!(worker_sync.join(pending_name).is_file());
        assert!(worker_sync.join(retained_name).is_file());
    }

    #[test]
    fn campaign_processing_writes_worker_file_immediately() {
        let corpus_root = temp_corpus_dir();
        let worker_subdir = corpus_root.join("worker1");
        let mut manager = empty_worker_corpus(1, corpus_root);

        manager.process_inputs_for_campaign(&[basic_tx()], &[], true, None);

        assert_eq!(manager.in_memory_corpus.len(), 1);
        assert_eq!(manager.metrics.corpus_count, 1);
        assert_eq!(read_corpus_dir(&worker_subdir.join(CORPUS_DIR)).count(), 1);
    }

    /// `RawCallResult` carrying a single edge hit, to drive `merge_edge_coverage` without the EVM.
    fn edge_call(edge: EdgeKey, count: u8) -> RawCallResult {
        RawCallResult {
            edge_coverage: Some(EdgeCoverage::CollisionFree(vec![EdgeCovHit { edge, count }])),
            ..Default::default()
        }
    }

    #[test]
    fn merge_edge_coverage_advances_timer_only_for_new_edges() {
        let corpus_root = temp_corpus_dir();
        let mut manager = empty_worker_corpus(1, corpus_root);

        // No edge seen yet.
        assert!(manager.time_since_new_edge().is_none());
        assert_eq!(manager.metrics.cumulative_edges_seen, 0);

        let edge =
            EdgeKey { address: Address::ZERO, depth: None, pc: 0, jump_dest: U256::from(10) };

        // First-time edge starts the timer.
        assert!(manager.merge_edge_coverage(&mut edge_call(edge, 1)));
        let first = manager.last_new_edge_at.expect("timer set after first new edge");
        assert_eq!(manager.metrics.cumulative_edges_seen, 1);

        // Same edge, higher bucket = a feature, not an edge: timer must not advance.
        assert!(manager.merge_edge_coverage(&mut edge_call(edge, 8)));
        assert_eq!(manager.last_new_edge_at, Some(first));
        assert_eq!(manager.metrics.cumulative_edges_seen, 1);
        assert_eq!(manager.metrics.cumulative_features_seen, 1);

        // A distinct edge advances the timer.
        let other =
            EdgeKey { address: Address::ZERO, depth: None, pc: 1, jump_dest: U256::from(20) };
        assert!(manager.merge_edge_coverage(&mut edge_call(other, 1)));
        let second = manager.last_new_edge_at.expect("timer present");
        assert!(second >= first);
        assert_eq!(manager.metrics.cumulative_edges_seen, 2);
        assert!(manager.time_since_new_edge().is_some());
    }

    #[test]
    fn empty_input_sequence_with_new_coverage_does_not_panic_or_insert() {
        // A run where every executed call was discarded (magic assume) or popped (reverts
        // without `fail_on_revert`, handler assertions) leaves no surviving inputs, yet
        // `new_coverage` can still be true because edge coverage is collected before the
        // input is popped. Processing must not panic and must not persist an entry.
        let corpus_root = temp_corpus_dir();
        let worker_subdir = corpus_root.join("worker1");
        let mut manager = empty_worker_corpus(1, corpus_root);

        manager.process_inputs_for_campaign(&[], &[], true, None);

        assert_eq!(manager.in_memory_corpus.len(), 0);
        assert_eq!(manager.metrics.corpus_count, 0);
        assert_eq!(read_corpus_dir(&worker_subdir.join(CORPUS_DIR)).count(), 0);

        // Live processing path must also tolerate the empty sequence.
        manager.process_inputs(&[], &[], true, None);
        assert_eq!(manager.in_memory_corpus.len(), 0);
        assert_eq!(read_corpus_dir(&worker_subdir.join(CORPUS_DIR)).count(), 0);
    }

    #[test]
    fn campaign_processing_defers_only_optimization_persistence() {
        let corpus_root = temp_corpus_dir();
        let mut manager = empty_worker_corpus(1, corpus_root.clone());
        let sequence = vec![basic_tx()];
        manager.process_inputs_for_campaign(
            &sequence,
            &[],
            false,
            Some((I256::try_from(7).unwrap(), sequence.clone())),
        );

        let worker_corpus_dir = corpus_root.join("worker1").join(CORPUS_DIR);
        let entries = read_corpus_dir(&worker_corpus_dir).collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        let persisted_sequence = entries[0].read_tx_seq().unwrap();
        assert_eq!(persisted_sequence.len(), sequence.len());
        assert_eq!(persisted_sequence[0].sender, sequence[0].sender);
        assert_eq!(persisted_sequence[0].call_details.target, sequence[0].call_details.target);
        assert_eq!(persisted_sequence[0].call_details.calldata, sequence[0].call_details.calldata);
        assert!(!corpus_root.join(OPTIMIZATION_BEST_FILE).exists());

        persist_campaign_optimization(
            &corpus_config(corpus_root.clone()),
            Some(I256::try_from(7).unwrap()),
            &sequence,
        );

        let state: OptimizationState =
            foundry_common::fs::read_json_file(&corpus_root.join(OPTIMIZATION_BEST_FILE)).unwrap();
        assert_eq!(state.best_value, I256::try_from(7).unwrap());
        assert_eq!(state.best_sequence.len(), sequence.len());
        assert_eq!(state.best_sequence[0].sender, sequence[0].sender);
        assert_eq!(state.best_sequence[0].call_details.target, sequence[0].call_details.target);
        assert_eq!(state.best_sequence[0].call_details.calldata, sequence[0].call_details.calldata);
    }

    #[test]
    fn persisted_worker_corpus_entries_are_deduped_by_uuid() {
        let corpus_root = temp_corpus_dir();
        let corpus = CorpusEntry::new(vec![basic_tx()]);
        let duplicate = corpus.clone();

        let worker0_corpus = corpus_root.join("worker0").join(CORPUS_DIR);
        let worker1_corpus = corpus_root.join("worker1").join(CORPUS_DIR);
        fs::create_dir_all(&worker0_corpus).unwrap();
        fs::create_dir_all(&worker1_corpus).unwrap();
        corpus.write_to_disk_in(&worker0_corpus, false).unwrap();
        duplicate.write_to_disk_in(&worker1_corpus, false).unwrap();

        let mut seen = HashSet::new();
        let entries = unique_corpus_entries(&canonical_replay_dirs(&corpus_root), &mut seen)
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].uuid, corpus.uuid);
    }

    #[test]
    fn corpus_entry_write_uses_unparsable_temp_file() {
        let corpus_dir = temp_corpus_dir();
        let corpus = CorpusEntry::new(vec![basic_tx()]);
        let temp_path =
            corpus_dir.join(format!(".{}.{}.tmp", corpus.file_name(false), Uuid::new_v4()));
        fs::write(&temp_path, b"{").unwrap();

        let path = corpus.write_to_disk_in(&corpus_dir, false).unwrap();
        let entries = read_corpus_dir(&corpus_dir).collect::<Vec<_>>();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, path);
        assert!(temp_path.exists());
    }

    #[test]
    fn persist_corpus_seed_skips_duplicate_sequence() {
        let corpus_root = temp_corpus_dir();
        let config = corpus_config(corpus_root.clone());
        let sequence = vec![basic_tx_with_calldata(vec![0x12, 0x34])];

        let first = persist_corpus_seed(&config, sequence.clone()).unwrap().unwrap();
        let second = persist_corpus_seed(&config, sequence).unwrap().unwrap();
        let entries =
            read_corpus_dir(&corpus_root.join("worker0").join(CORPUS_DIR)).collect::<Vec<_>>();

        assert_eq!(first, second);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn non_master_campaign_worker_uses_persisted_optimization_baseline() {
        let corpus_root = temp_corpus_dir();
        let persisted_sequence = vec![basic_tx()];
        let persisted_state = OptimizationState {
            best_value: I256::try_from(100).unwrap(),
            best_sequence: persisted_sequence,
        };
        foundry_common::fs::write_json_file(
            &corpus_root.join(OPTIMIZATION_BEST_FILE),
            &persisted_state,
        )
        .unwrap();
        let config = corpus_config(corpus_root);
        let generator =
            test_sequence(&config, TxGenerator::from_strategy(Just(basic_tx()).boxed()));
        let mut manager = WorkerCorpus::new::<foundry_evm_core::evm::EthEvmNetwork>(
            1,
            config,
            generator,
            None,
            None,
            ReplayTarget { stateless: None, fuzzed_contracts: None, dynamic: None },
        )
        .unwrap();

        let worse_sequence = vec![basic_tx()];
        manager.process_inputs_for_campaign(
            &worse_sequence,
            &[],
            false,
            Some((I256::try_from(50).unwrap(), worse_sequence.clone())),
        );

        let better_sequence = vec![basic_tx()];
        manager.process_inputs_for_campaign(
            &better_sequence,
            &[],
            false,
            Some((I256::try_from(150).unwrap(), better_sequence.clone())),
        );
        assert_eq!(manager.optimization_best_value, Some(I256::try_from(150).unwrap()));
    }

    #[test]
    fn worker_can_initialize_from_warmed_seed() {
        let corpus_root = temp_corpus_dir();
        let tx_seq = vec![basic_tx()];
        let seed = WorkerCorpusSeed {
            in_memory_corpus: vec![CorpusEntry::new(tx_seq.clone())],
            history_map: vec![1, 2, 3],
            edge_indices: EdgeIndexMap::default(),
            sancov_history_map: vec![4, 5],
            metrics: CorpusMetrics {
                cumulative_edges_seen: 7,
                cumulative_features_seen: 11,
                corpus_count: 1,
                favored_items: 0,
            },
            replay_dirs: None,
            failed_replays: 13,
            optimization_best_value: Some(I256::try_from(17).unwrap()),
            optimization_best_sequence: tx_seq,
            last_new_edge_at: None,
        };

        let config = corpus_config(corpus_root);
        let generator =
            test_sequence(&config, TxGenerator::from_strategy(Just(basic_tx()).boxed()));
        let manager = WorkerCorpus::from_seed(1, config, generator, seed).unwrap();

        assert_eq!(manager.in_memory_corpus.len(), 1);
        assert_eq!(manager.history_map, vec![1, 2, 3]);
        assert_eq!(manager.sancov_history_map, vec![4, 5]);
        assert_eq!(manager.metrics.cumulative_edges_seen, 7);
        assert_eq!(manager.metrics.cumulative_features_seen, 11);
        assert_eq!(manager.metrics.corpus_count, 1);
        assert_eq!(manager.failed_replays, 13);
        let (value, sequence) = manager.optimization_initial_state();
        assert_eq!(value, Some(I256::try_from(17).unwrap()));
        assert_eq!(sequence.len(), 1);
    }

    #[test]
    fn clone_for_worker_shards_warmed_corpus_and_recomputes_metrics() {
        let entries = (0..10)
            .map(|idx| {
                let mut entry = CorpusEntry::new(vec![basic_tx()]);
                entry.is_favored = idx % 2 == 0;
                entry
            })
            .collect::<Vec<_>>();
        let entry_ids = entries.iter().map(|entry| entry.uuid).collect::<Vec<_>>();
        let seed = WorkerCorpusSeed {
            in_memory_corpus: entries,
            history_map: vec![1, 2, 3],
            edge_indices: EdgeIndexMap::default(),
            sancov_history_map: vec![4, 5],
            metrics: CorpusMetrics {
                cumulative_edges_seen: 7,
                cumulative_features_seen: 11,
                corpus_count: 10,
                favored_items: 5,
            },
            replay_dirs: None,
            failed_replays: 13,
            optimization_best_value: Some(I256::try_from(17).unwrap()),
            optimization_best_sequence: vec![basic_tx()],
            last_new_edge_at: None,
        };

        let worker_count = 3;
        let shards = (0..worker_count)
            .map(|worker_id| seed.clone_for_worker(worker_id, worker_count, true))
            .collect::<Vec<_>>();
        let mut sharded_ids = shards
            .iter()
            .flat_map(|shard| shard.in_memory_corpus.iter().map(|entry| entry.uuid))
            .collect::<Vec<_>>();
        let mut expected_ids = entry_ids.clone();
        sharded_ids.sort_unstable();
        expected_ids.sort_unstable();

        assert_eq!(sharded_ids, expected_ids);
        assert_eq!(
            shards[0].in_memory_corpus.iter().map(|entry| entry.uuid).collect::<Vec<_>>(),
            [entry_ids[0], entry_ids[3], entry_ids[6], entry_ids[9]]
        );
        assert_eq!(
            shards[1].in_memory_corpus.iter().map(|entry| entry.uuid).collect::<Vec<_>>(),
            [entry_ids[1], entry_ids[4], entry_ids[7]]
        );
        assert_eq!(
            shards[2].in_memory_corpus.iter().map(|entry| entry.uuid).collect::<Vec<_>>(),
            [entry_ids[2], entry_ids[5], entry_ids[8]]
        );
        assert_eq!(
            shards.iter().map(|shard| shard.in_memory_corpus.len()).collect::<Vec<_>>(),
            [4, 3, 3]
        );
        assert_eq!(
            shards.iter().map(|shard| shard.metrics.corpus_count).collect::<Vec<_>>(),
            [4, 3, 3]
        );
        assert_eq!(
            shards.iter().map(|shard| shard.metrics.favored_items).collect::<Vec<_>>(),
            [2, 1, 2]
        );
        assert!(shards.iter().all(|shard| shard.history_map == seed.history_map));
        assert!(shards.iter().all(|shard| shard.sancov_history_map == seed.sancov_history_map));
        assert!(shards.iter().all(|shard| shard.metrics.cumulative_edges_seen == 7));
        assert!(shards.iter().all(|shard| shard.metrics.cumulative_features_seen == 11));
    }

    #[test]
    fn clone_for_worker_can_strip_cmp_sequences() {
        let cmp = CmpOperands {
            op1: U256::from(1),
            op2: U256::from(2),
            pc: 3,
            address: Address::ZERO,
            opcode: 0,
        };
        let entries = (0..2)
            .map(|_| {
                CorpusEntry::new_with_cmp(
                    vec![basic_tx()],
                    vec![vec![ComparisonHint { lhs: cmp.op1, rhs: cmp.op2 }]],
                    Uuid::new_v4(),
                )
            })
            .collect::<Vec<_>>();
        let seed = WorkerCorpusSeed { in_memory_corpus: entries, ..Default::default() };

        let with_cmp = seed.clone_for_worker(0, 1, true);
        let without_cmp = seed.clone_for_worker(0, 1, false);

        assert!(with_cmp.in_memory_corpus.iter().all(|entry| !entry.cmp_seq[0].is_empty()));
        assert!(without_cmp.in_memory_corpus.iter().all(|entry| entry.cmp_seq.is_empty()));
    }

    #[test]
    fn retain_replayable_removes_off_target_corpus_entries() {
        let target = Address::from([0x11; 20]);
        let foo = Function::parse("foo()").unwrap();
        let bar = Function::parse("bar()").unwrap();
        let foo_selector = foo.selector();
        let foo_tx = tx_for_function(target, &foo, &[]);
        let bar_tx = tx_for_function(target, &bar, &[]);
        let mut foo_entry = CorpusEntry::new(vec![foo_tx.clone()]);
        foo_entry.is_favored = true;
        let mut bar_entry = CorpusEntry::new(vec![bar_tx.clone()]);
        bar_entry.is_favored = true;
        let mut seed = WorkerCorpusSeed {
            in_memory_corpus: vec![foo_entry, bar_entry],
            metrics: CorpusMetrics { corpus_count: 2, favored_items: 2, ..Default::default() },
            optimization_best_value: Some(I256::try_from(17).unwrap()),
            optimization_best_sequence: vec![bar_tx],
            ..Default::default()
        };
        let targeted_contracts =
            targeted_contracts_with_selective_functions(target, vec![foo, bar], [foo_selector]);
        let targets = targeted_contracts.targets();

        seed.retain_replayable(&targets);

        assert_eq!(seed.in_memory_corpus.len(), 1);
        assert_eq!(seed.in_memory_corpus[0].tx_seq.len(), 1);
        assert_eq!(
            seed.in_memory_corpus[0].tx_seq[0].call_details.target,
            foo_tx.call_details.target
        );
        assert_eq!(
            seed.in_memory_corpus[0].tx_seq[0].call_details.calldata,
            foo_tx.call_details.calldata
        );
        assert_eq!(seed.metrics.corpus_count, 1);
        assert_eq!(seed.metrics.favored_items, 1);
        assert!(seed.optimization_best_value.is_none());
        assert!(seed.optimization_best_sequence.is_empty());
    }

    #[test]
    fn hoist_observed_calls_bundles_replayable_subcalls_into_one_corpus_entry() {
        let target = Address::from([0x42; 20]);
        let other = Address::from([0x43; 20]);
        let sender = Address::from([0xaa; 20]);
        let observed_caller = Address::from([0xbb; 20]);
        let foo = Function::parse("foo(uint256)").unwrap();
        let bar = Function::parse("bar()").unwrap();
        let foo_selector = foo.selector();
        let bar_selector = bar.selector();
        let targeted_contracts = targeted_contracts_with_selective_functions(
            target,
            vec![foo, bar],
            [foo_selector, bar_selector],
        );

        let mut foo_calldata = vec![0u8; 36];
        foo_calldata[..4].copy_from_slice(&foo_selector[..]);
        let bar_calldata = bar_selector.to_vec();
        let mut unknown_selector = vec![0u8; 36];
        unknown_selector[..4].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        let value = U256::from(1);

        let observed = vec![
            ObservedCall {
                depth: 1,
                caller: observed_caller,
                target: other,
                calldata: Bytes::from(foo_calldata.clone()),
                value: Some(value),
            },
            ObservedCall {
                depth: 1,
                caller: observed_caller,
                target,
                calldata: Bytes::from(foo_calldata),
                value: None,
            },
            ObservedCall {
                depth: 2,
                caller: observed_caller,
                target,
                calldata: Bytes::from(bar_calldata),
                value: None,
            },
            ObservedCall {
                depth: 1,
                caller: observed_caller,
                target,
                calldata: Bytes::from(unknown_selector),
                value: None,
            },
            ObservedCall {
                depth: 1,
                caller: observed_caller,
                target,
                calldata: Bytes::from(vec![0u8; 3]),
                value: None,
            },
        ];
        let parent_tx = BasicTxDetails {
            warp: Some(U256::from(123)),
            roll: Some(U256::from(456)),
            sender,
            call_details: CallDetails {
                target: Address::from([0x99; 20]),
                calldata: Bytes::new(),
                value: None,
            },
        };
        let mut manager = empty_worker_corpus(0, temp_corpus_dir());

        manager.hoist_observed_calls(
            &observed,
            &parent_tx,
            &targeted_contracts,
            CorpusInsertionMode::Live,
        );

        assert_eq!(manager.in_memory_corpus.len(), 1);
        assert_eq!(manager.metrics.corpus_count, 1);

        let entry = manager.in_memory_corpus.last().unwrap();
        assert_eq!(entry.tx_seq.len(), 2);
        let tx = &entry.tx_seq[0];
        assert_eq!(tx.warp, parent_tx.warp);
        assert_eq!(tx.roll, parent_tx.roll);
        assert_eq!(tx.sender, observed_caller);
        assert_eq!(tx.call_details.target, target);
        assert_eq!(&tx.call_details.calldata[..4], &foo_selector[..]);
        assert_eq!(tx.call_details.value, None);

        let tx = &entry.tx_seq[1];
        assert_eq!(tx.warp, None);
        assert_eq!(tx.roll, None);
        assert_eq!(tx.sender, observed_caller);
        assert_eq!(tx.call_details.target, target);
        assert_eq!(&tx.call_details.calldata[..4], &bar_selector[..]);
        assert_eq!(tx.call_details.value, None);
    }

    #[test]
    fn hoist_observed_calls_persists_immediately() {
        let target = Address::from([0x42; 20]);
        let foo = Function::parse("foo()").unwrap();
        let selector = foo.selector();
        let targeted_contracts = targeted_contracts_with_selective_functions(target, vec![foo], []);
        let observed = vec![ObservedCall {
            depth: 1,
            caller: Address::from([0xaa; 20]),
            target,
            calldata: Bytes::from(selector.to_vec()),
            value: None,
        }];
        let corpus_root = temp_corpus_dir();
        let worker_corpus_dir = corpus_root.join("worker1").join(CORPUS_DIR);
        let mut manager = empty_worker_corpus(1, corpus_root);

        manager.hoist_observed_calls(
            &observed,
            &basic_tx(),
            &targeted_contracts,
            CorpusInsertionMode::Live,
        );

        assert_eq!(manager.in_memory_corpus.len(), 1);
        assert_eq!(read_corpus_dir(&worker_corpus_dir).count(), 1);
    }

    #[test]
    fn hoist_observed_calls_skips_empty_or_non_coverage_guided_inputs() {
        let target = Address::from([0x42; 20]);
        let foo = Function::parse("foo()").unwrap();
        let selector = foo.selector();
        let targeted_contracts = targeted_contracts_with_selective_functions(target, vec![foo], []);
        let observed = vec![ObservedCall {
            depth: 1,
            caller: Address::from([0xaa; 20]),
            target,
            calldata: Bytes::from(selector.to_vec()),
            value: None,
        }];

        let mut no_corpus_config = corpus_config(temp_corpus_dir());
        no_corpus_config.corpus_dir = None;
        let generator =
            test_sequence(&no_corpus_config, TxGenerator::from_strategy(Just(basic_tx()).boxed()));
        let mut manager =
            WorkerCorpus::from_seed(0, no_corpus_config, generator, WorkerCorpusSeed::default())
                .unwrap();
        manager.hoist_observed_calls(
            &observed,
            &basic_tx(),
            &targeted_contracts,
            CorpusInsertionMode::Live,
        );
        assert!(manager.in_memory_corpus.is_empty());

        let mut manager = empty_worker_corpus(0, temp_corpus_dir());
        manager.hoist_observed_calls(
            &[],
            &basic_tx(),
            &targeted_contracts,
            CorpusInsertionMode::Live,
        );
        assert!(manager.in_memory_corpus.is_empty());
    }

    #[test]
    fn sequence_from_observed_keeps_only_direct_replayable_calls() {
        let target = Address::from([0x42; 20]);
        let other = Address::from([0x43; 20]);
        let sender = Address::from([0xaa; 20]);
        let nested_caller = Address::from([0xbb; 20]);
        let foo = Function::parse("foo(uint256)").unwrap();
        let bar = Function::parse("bar()").unwrap();
        let foo_selector = foo.selector();
        let bar_selector = bar.selector();
        let targeted_contracts =
            targeted_contracts_with_selective_functions(target, vec![foo, bar], [foo_selector]);
        let targets = targeted_contracts.targets();

        let mut foo_calldata = vec![0u8; 36];
        foo_calldata[..4].copy_from_slice(&foo_selector[..]);
        let bar_calldata = bar_selector.to_vec();
        let observed = vec![
            ObservedCall {
                depth: 1,
                caller: sender,
                target,
                calldata: Bytes::from(foo_calldata.clone()),
                value: None,
            },
            ObservedCall {
                depth: 2,
                caller: nested_caller,
                target,
                calldata: Bytes::from(foo_calldata),
                value: None,
            },
            ObservedCall {
                depth: 1,
                caller: sender,
                target,
                calldata: Bytes::from(bar_calldata),
                value: None,
            },
            ObservedCall {
                depth: 1,
                caller: sender,
                target: other,
                calldata: Bytes::from(foo_selector.to_vec()),
                value: None,
            },
        ];

        let seq = sequence_from_observed(&observed, &targets, ObservedCallDepth::DirectOnly, None);

        assert_eq!(seq.len(), 1);
        assert_eq!(seq[0].sender, sender);
        assert_eq!(seq[0].call_details.target, target);
        assert_eq!(&seq[0].call_details.calldata[..4], &foo_selector[..]);
    }

    #[test]
    fn push_observed_sequence_live_persists_and_memory_only_does_not() {
        let corpus_root = temp_corpus_dir();
        let worker0_corpus_dir = corpus_root.join("worker0").join(CORPUS_DIR);
        let mut manager = empty_worker_corpus(0, corpus_root.clone());

        manager.push_observed_sequence(vec![basic_tx()], CorpusInsertionMode::Live);
        assert_eq!(manager.in_memory_corpus.len(), 1);
        assert_eq!(read_corpus_dir(&worker0_corpus_dir).count(), 1);

        let mut manager = empty_worker_corpus(1, corpus_root.clone());
        let worker1_corpus_dir = corpus_root.join("worker1").join(CORPUS_DIR);
        manager.push_observed_sequence(vec![basic_tx()], CorpusInsertionMode::MemoryOnly);
        assert_eq!(manager.in_memory_corpus.len(), 1);
        assert_eq!(read_corpus_dir(&worker1_corpus_dir).count(), 0);
    }

    #[test]
    fn detects_legacy_invariant_corpus_dirs_without_matching_worker_dirs() {
        let corpus_root = temp_corpus_dir();
        fs::create_dir_all(corpus_root.join("worker0")).unwrap();
        assert!(!has_legacy_invariant_corpus_dirs(&corpus_root));

        fs::create_dir_all(corpus_root.join("invariant_a")).unwrap();
        assert!(has_legacy_invariant_corpus_dirs(&corpus_root));
    }

    #[test]
    fn ignores_optimization_invariant_corpus_dirs_when_detecting_legacy_dirs() {
        let corpus_root = temp_corpus_dir();
        fs::create_dir_all(corpus_root.join("worker0")).unwrap();
        let optimization_dir = corpus_root.join("invariant_optimize");
        fs::create_dir_all(optimization_dir.join("worker0")).unwrap();
        fs::write(optimization_dir.join(OPTIMIZATION_BEST_FILE), "{}").unwrap();

        assert!(!has_legacy_invariant_corpus_dirs(&corpus_root));

        fs::create_dir_all(corpus_root.join("invariant_legacy").join("worker0")).unwrap();
        assert!(has_legacy_invariant_corpus_dirs(&corpus_root));
    }

    #[test]
    fn favored_sets_true_and_metrics_increment_when_ratio_gt_threshold() {
        let (mut manager, uuid) = new_manager_with_single_corpus();
        let corpus = manager.in_memory_corpus.iter_mut().find(|c| c.uuid == uuid).unwrap();
        corpus.total_mutations = 4;
        corpus.new_finds_produced = 2; // ratio currently 0.5 if both increment → 3/5 = 0.6 > 0.3.
        corpus.is_favored = false;

        // Ensure metrics start at 0.
        assert_eq!(manager.metrics.favored_items, 0);

        // Mark this as the currently mutated corpus and process a run with new coverage.
        manager.current_mutated_index = Some(0);
        manager.process_inputs(&[basic_tx()], &[], true, None);

        let corpus = manager.in_memory_corpus.iter().find(|c| c.uuid == uuid).unwrap();
        assert!(corpus.is_favored, "expected favored to be true when ratio > threshold");
        assert_eq!(
            manager.metrics.favored_items, 1,
            "favored_items should increment on false→true"
        );
    }

    #[test]
    fn favored_sets_false_and_metrics_decrement_when_ratio_lt_threshold() {
        let (mut manager, uuid) = new_manager_with_single_corpus();
        let corpus = manager.in_memory_corpus.iter_mut().find(|c| c.uuid == uuid).unwrap();
        corpus.total_mutations = 9;
        corpus.new_finds_produced = 3; // 3/9 = 0.333.. > 0.3; after +1: 3/10 = 0.3 => not favored.
        corpus.is_favored = true; // Start as favored.

        manager.metrics.favored_items = 1;

        // Next run does NOT produce coverage → only total_mutations increments, ratio drops.
        manager.current_mutated_index = Some(0);
        manager.process_inputs(&[basic_tx()], &[], false, None);

        let corpus = manager.in_memory_corpus.iter().find(|c| c.uuid == uuid).unwrap();
        assert!(!corpus.is_favored, "expected favored to be false when ratio < threshold");
        assert_eq!(
            manager.metrics.favored_items, 0,
            "favored_items should decrement on true→false"
        );
    }

    #[test]
    fn favored_is_false_on_ratio_equal_threshold() {
        let (mut manager, uuid) = new_manager_with_single_corpus();
        let corpus = manager.in_memory_corpus.iter_mut().find(|c| c.uuid == uuid).unwrap();
        // After this call with new_coverage=true, totals become 10 and 3 → 0.3.
        corpus.total_mutations = 9;
        corpus.new_finds_produced = 2;
        corpus.is_favored = false;

        manager.current_mutated_index = Some(0);
        manager.process_inputs(&[basic_tx()], &[], true, None);

        let corpus = manager.in_memory_corpus.iter().find(|c| c.uuid == uuid).unwrap();
        assert!(
            !(corpus.is_favored),
            "with strict '>' comparison, favored must be false when ratio == threshold"
        );
    }

    #[test]
    fn eviction_skips_favored_and_evicts_non_favored() {
        // Manager with two corpora.
        let mut favored = CorpusEntry::new(vec![basic_tx()]);
        favored.total_mutations = 2;
        favored.is_favored = true;

        let mut non_favored = CorpusEntry::new(vec![basic_tx()]);
        non_favored.total_mutations = 2;
        non_favored.is_favored = false;
        let non_favored_uuid = non_favored.uuid;

        let mut manager = seeded_worker_corpus(0, temp_corpus_dir(), vec![favored, non_favored]);

        // First eviction should remove the non-favored one.
        manager.evict_oldest_corpus().unwrap();
        assert_eq!(manager.in_memory_corpus.len(), 1);
        assert!(manager.in_memory_corpus.iter().all(|c| c.is_favored));

        // Attempt eviction again: only favored remains → should not remove.
        manager.evict_oldest_corpus().unwrap();
        assert_eq!(manager.in_memory_corpus.len(), 1, "favored corpus must not be evicted");

        // Ensure the evicted one was the non-favored uuid.
        assert!(manager.in_memory_corpus.iter().all(|c| c.uuid != non_favored_uuid));
    }

    #[test]
    fn non_synchronizing_entries_remain_evictable() {
        let corpus_root = temp_corpus_dir();
        let config = corpus_config(corpus_root);
        let generator =
            test_sequence(&config, TxGenerator::from_strategy(Just(basic_tx()).boxed()));
        let mut manager =
            WorkerCorpus::from_seed(0, config, generator, WorkerCorpusSeed::default()).unwrap();
        let mut evictable = CorpusEntry::new(vec![basic_tx_with_calldata([1])]);
        evictable.total_mutations = 1;
        let evictable_uuid = evictable.uuid;
        manager.push_corpus_entry(evictable);
        manager.push_corpus_entry(CorpusEntry::new(vec![basic_tx_with_calldata([2])]));

        manager.evict_oldest_corpus().unwrap();

        assert_eq!(manager.in_memory_corpus.len(), 1);
        assert!(manager.in_memory_corpus.iter().all(|entry| entry.uuid != evictable_uuid));
        assert!(manager.new_entry_indices.is_empty());
    }
}
