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

use super::corpus_io::{CorpusDirEntry, canonical_replay_dirs, read_corpus_dir};
use crate::{
    executors::{Executor, RawCallResult, invariant::execute_tx},
    inspectors::{CmpOperands, EdgeIndexMap, MAX_EDGE_COUNT},
};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes, I256, U256};
use eyre::{Result, eyre};
use foundry_common::{ContractsByAddress, ContractsByArtifact, TestFunctionExt, sh_warn};
use foundry_config::{FuzzCorpusConfig, FuzzCorpusMutationWeights};
use foundry_evm_core::{constants::CALLER, evm::FoundryEvmNetwork, utils::StateChangeset};
use foundry_evm_fuzz::{
    BasicTxDetails, CallDetails, ObservedCall,
    invariant::{
        ArtifactFilters, FuzzRunIdentifiedContracts, InvariantContract, TargetedContracts,
    },
    strategies::{
        EvmFuzzState, FuzzStateReader, InvariantFuzzState, generate_msg_value, mutate_param_value,
    },
};
use proptest::{
    prelude::{Rng, Strategy},
    strategy::{BoxedStrategy, ValueTree},
    test_runner::TestRunner,
};
use rand::distr::{Distribution, weighted::WeightedIndex};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fmt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
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

fn weighted_arg_mutation(
    rng: &mut impl Rng,
    distribution: Option<&WeightedIndex<u32>>,
) -> Option<bool> {
    distribution.map(|distribution| distribution.sample(rng) == 1)
}

fn weighted_mutation_type(rng: &mut impl Rng, distribution: &WeightedIndex<u32>) -> MutationType {
    match distribution.sample(rng) {
        0 => MutationType::Splice,
        1 => MutationType::Repeat,
        2 => MutationType::Interleave,
        3 => MutationType::Prefix,
        4 => MutationType::Suffix,
        5 => MutationType::Abi,
        6 => MutationType::Cmp,
        _ => unreachable!("mutation distribution only has seven entries"),
    }
}

fn validate_supported_mutation_weight_total(
    mutation_weights: FuzzCorpusMutationWeights,
) -> Result<()> {
    let total = mutation_weights.total();
    if total > u64::from(u32::MAX) {
        return Err(eyre!(
            "effective mutation weights sum to {total}, which exceeds the maximum supported \
             total {}",
            u32::MAX
        ));
    }

    Ok(())
}

/// Possible mutation strategies to apply on a call sequence.
#[derive(Debug, Clone)]
enum MutationType {
    /// Splice original call sequence.
    Splice,
    /// Repeat selected call several times.
    Repeat,
    /// Interleave calls from two random call sequences.
    Interleave,
    /// Replace prefix of the original call sequence with new calls.
    Prefix,
    /// Replace suffix of the original call sequence with new calls.
    Suffix,
    /// ABI mutate random args of selected call in sequence.
    Abi,
    /// Replace input bytes using comparison operands observed for a corpus entry
    /// (input-to-state, LibAFL-style).
    Cmp,
}

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
    cmp_seq: Vec<Vec<CmpOperands>>,
    // Whether this corpus is favored, i.e. producing new finds more often than
    // `FAVORABILITY_THRESHOLD`.
    is_favored: bool,
    /// Timestamp of when this entry was written to disk in seconds.
    #[serde(skip_serializing)]
    timestamp: u64,
}

impl CorpusEntry {
    /// Creates a corpus entry with a new UUID.
    pub fn new(tx_seq: Vec<BasicTxDetails>) -> Self {
        Self::new_with_cmp(tx_seq, Vec::new(), Uuid::new_v4())
    }

    /// Creates a corpus entry with the given UUID and per-call cmp operand log.
    pub fn new_with_cmp(
        tx_seq: Vec<BasicTxDetails>,
        cmp_seq: Vec<Vec<CmpOperands>>,
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
        }
    }

    fn write_to_disk_in(&self, dir: &Path, can_gzip: bool) -> foundry_common::fs::Result<()> {
        let file_name = self.file_name(can_gzip);
        let path = dir.join(file_name);
        if self.should_gzip(can_gzip) {
            foundry_common::fs::write_json_gzip_file(&path, &self.tx_seq)
        } else {
            foundry_common::fs::write_json_file(&path, &self.tx_seq)
        }
    }

    fn file_name(&self, can_gzip: bool) -> String {
        let ext = if self.should_gzip(can_gzip) { ".json.gz" } else { ".json" };
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

/// Corpus entry selected by a worker and returned for logical-campaign persistence.
#[derive(Debug, Clone)]
pub(crate) struct CampaignCorpusEntry {
    tx_seq: Vec<BasicTxDetails>,
    dedupe_by_coverage: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CorpusInsertionMode {
    Live,
    Deferred,
    MemoryOnly,
}

struct ReplayOutcome {
    keep_entry: bool,
    new_coverage: bool,
    /// Whether replay hit a first-time edge (advances the per-worker "time since new edge" timer).
    new_edge: bool,
    cmp_seq: Vec<Vec<CmpOperands>>,
    failed_replays: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct ReplayTarget<'a> {
    pub(crate) fuzzed_function: Option<&'a Function>,
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
            failed_replays: self.failed_replays,
            optimization_best_value: self.optimization_best_value,
            optimization_best_sequence: self.optimization_best_sequence.clone(),
            last_new_edge_at: self.last_new_edge_at,
        }
    }

    pub(crate) fn load_from_disk<FEN: FoundryEvmNetwork>(
        config: &FuzzCorpusConfig,
        executor: Option<&Executor<FEN>>,
        fuzzed_function: Option<&Function>,
        fuzzed_contracts: Option<&FuzzRunIdentifiedContracts>,
        dynamic: Option<DynamicTargetCtx<'_>>,
    ) -> Result<Self> {
        let mut seed = Self::empty(config).with_optimization_state(config);
        let Some(corpus_dir) = &config.corpus_dir else {
            return Ok(seed);
        };

        // Seed in-memory corpus with the persisted optimization best sequence so the mutation
        // engine can build on it in future runs.
        if !seed.optimization_best_sequence.is_empty() {
            seed.in_memory_corpus.push(CorpusEntry::new(seed.optimization_best_sequence.clone()));
            seed.metrics.corpus_count += 1;
        }

        if fuzzed_contracts.is_some() && has_legacy_invariant_corpus_dirs(corpus_dir) {
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
        let target = ReplayTarget { fuzzed_function, fuzzed_contracts, dynamic: dynamic.as_ref() };
        for entry in unique_corpus_entries(&canonical_replay_dirs(corpus_dir), &mut seen_entries) {
            // A corrupt or truncated corpus file (e.g. a process killed mid-write, since entries
            // are persisted non-atomically) must not abort the whole campaign startup: skip it
            // and keep loading the rest of the corpus.
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

    /// Filters and persists logical-campaign corpus entries after worker results have merged.
    ///
    /// This consumes the deferred entries and writes each retained entry as soon as replay proves
    /// it contributes new coverage. Keeping this path streaming avoids building a second filtered
    /// copy of every campaign entry during invariant finalization.
    pub(crate) fn persist_filtered_campaign_outputs<FEN: FoundryEvmNetwork>(
        &self,
        config: &FuzzCorpusConfig,
        entries: impl IntoIterator<Item = CampaignCorpusEntry>,
        executor: &Executor<FEN>,
        target: ReplayTarget<'_>,
        optimization_best: Option<(I256, &[BasicTxDetails])>,
    ) -> Result<()> {
        let mut history_map = self.history_map.clone();
        let mut edge_indices = self.edge_indices.clone();
        let mut sancov_history_map = self.sancov_history_map.clone();

        let mut output_dir_ready = false;
        for entry in entries {
            if entry.dedupe_by_coverage {
                let coverage = ReplayCoverage {
                    history_map: &mut history_map,
                    edge_indices: &mut edge_indices,
                    sancov_history_map: &mut sancov_history_map,
                    metrics: None,
                };
                let ReplayOutcome { keep_entry, new_coverage, .. } =
                    replay_corpus_sequence(&entry.tx_seq, executor, target, coverage)?;
                if !keep_entry || !new_coverage {
                    continue;
                }
            }

            if !output_dir_ready {
                prepare_campaign_output_dir(config);
                output_dir_ready = true;
            }
            persist_campaign_entry(config, entry);
        }

        persist_optimization_output(config, optimization_best);
        Ok(())
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
    /// Fuzzed calls generator.
    tx_generator: BoxedStrategy<BasicTxDetails>,
    /// Call sequence mutation weights used by stateful fuzzing.
    mutation_weights: FuzzCorpusMutationWeights,
    /// Weighted stateful sequence mutation distribution.
    mutation_distribution: WeightedIndex<u32>,
    /// Weighted ABI/CMP argument mutation distribution used by stateless fuzzing.
    arg_mutation_distribution: Option<WeightedIndex<u32>>,
    /// Identifier of current mutated entry for this worker.
    current_mutated_index: Option<usize>,
    /// Config
    config: Arc<FuzzCorpusConfig>,
    /// Indices of new entries added to [`WorkerCorpus::in_memory_corpus`] since last sync.
    new_entry_indices: Vec<usize>,
    /// Last sync timestamp in seconds.
    last_sync_timestamp: u64,
    /// Worker Dir
    /// corpus_dir/worker1/
    worker_dir: Option<PathBuf>,
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
        if WorkerCorpus::can_replay_tx(tx, target.fuzzed_function, target.fuzzed_contracts) {
            let mut call_result = execute_tx(executor, tx)?;
            cmp_seq.push(call_result.evm_cmp_values.take().unwrap_or_default());
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

            if reject_unmatched_function && target.fuzzed_function.is_some() {
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
    pub fn new<FEN: FoundryEvmNetwork>(
        id: usize,
        config: FuzzCorpusConfig,
        tx_generator: BoxedStrategy<BasicTxDetails>,
        // Only required by master worker (id = 0) to replay existing corpus.
        executor: Option<&Executor<FEN>>,
        fuzzed_function: Option<&Function>,
        fuzzed_contracts: Option<&FuzzRunIdentifiedContracts>,
        dynamic: Option<DynamicTargetCtx<'_>>,
    ) -> Result<Self> {
        let seed = if id == 0 {
            WorkerCorpusSeed::load_from_disk(
                &config,
                executor,
                fuzzed_function,
                fuzzed_contracts,
                dynamic,
            )?
        } else {
            WorkerCorpusSeed::empty(&config).with_optimization_state(&config)
        };
        Self::from_seed(id, config, tx_generator, seed)
    }

    pub(crate) fn from_seed(
        id: usize,
        config: FuzzCorpusConfig,
        tx_generator: BoxedStrategy<BasicTxDetails>,
        seed: WorkerCorpusSeed,
    ) -> Result<Self> {
        let mutation_weights = config.mutation_weights.effective();
        validate_supported_mutation_weight_total(mutation_weights)?;
        let mutation_distribution = WeightedIndex::new([
            mutation_weights.mutation_weight_splice,
            mutation_weights.mutation_weight_repeat,
            mutation_weights.mutation_weight_interleave,
            mutation_weights.mutation_weight_prefix,
            mutation_weights.mutation_weight_suffix,
            mutation_weights.mutation_weight_abi,
            mutation_weights.mutation_weight_cmp,
        ])
        .map_err(|err| eyre!("invalid corpus mutation weights: {err}"))?;
        let arg_mutation_distribution = if mutation_weights.mutation_weight_abi == 0
            && mutation_weights.mutation_weight_cmp == 0
        {
            None
        } else {
            Some(
                WeightedIndex::new([
                    mutation_weights.mutation_weight_abi,
                    mutation_weights.mutation_weight_cmp,
                ])
                .map_err(|err| eyre!("invalid argument mutation weights: {err}"))?,
            )
        };

        let worker_dir = config.corpus_dir.as_ref().map(|corpus_dir| {
            let worker_dir = corpus_dir.join(format!("{WORKER}{id}"));
            let worker_corpus = worker_dir.join(CORPUS_DIR);
            let sync_dir = worker_dir.join(SYNC_DIR);

            // Create the necessary directories for the worker.
            let _ = foundry_common::fs::create_dir_all(&worker_corpus);
            let _ = foundry_common::fs::create_dir_all(&sync_dir);

            worker_dir
        });

        Ok(Self {
            id,
            in_memory_corpus: seed.in_memory_corpus,
            history_map: seed.history_map,
            edge_indices: seed.edge_indices,
            sancov_history_map: seed.sancov_history_map,
            failed_replays: seed.failed_replays,
            metrics: seed.metrics,
            tx_generator,
            mutation_weights,
            mutation_distribution,
            arg_mutation_distribution,
            current_mutated_index: None,
            config: config.into(),
            new_entry_indices: Default::default(),
            last_sync_timestamp: 0,
            worker_dir,
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
        let _ = self.process_inputs_inner(inputs, cmp_seq, new_coverage, optimization, true);
    }

    /// Updates worker-local corpus state and returns any corpus entry to persist after the
    /// logical campaign has merged worker outputs.
    #[instrument(skip_all)]
    pub fn process_inputs_for_campaign(
        &mut self,
        inputs: &[BasicTxDetails],
        cmp_seq: &[Vec<CmpOperands>],
        new_coverage: bool,
        optimization: Option<(I256, Vec<BasicTxDetails>)>,
    ) -> Option<CampaignCorpusEntry> {
        self.process_inputs_inner(inputs, cmp_seq, new_coverage, optimization, false)
    }

    fn process_inputs_inner(
        &mut self,
        inputs: &[BasicTxDetails],
        cmp_seq: &[Vec<CmpOperands>],
        new_coverage: bool,
        optimization: Option<(I256, Vec<BasicTxDetails>)>,
        persist_now: bool,
    ) -> Option<CampaignCorpusEntry> {
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
            if persist_now {
                self.persist_optimization_state();
            }
        }

        if !self.config.is_coverage_guided() {
            return None;
        }

        // Collect inputs if current run produced new coverage or improved optimization.
        if !new_coverage && !improved_optimization {
            return None;
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
            return None;
        }
        let corpus_cmp_seq: Vec<Vec<CmpOperands>> =
            cmp_seq.iter().take(corpus_inputs.len()).cloned().collect();
        let corpus = CorpusEntry::new_with_cmp(corpus_inputs, corpus_cmp_seq, Uuid::new_v4());

        self.insert_corpus_entry(
            corpus,
            if persist_now { CorpusInsertionMode::Live } else { CorpusInsertionMode::Deferred },
            new_coverage,
        )
    }

    fn insert_corpus_entry(
        &mut self,
        corpus: CorpusEntry,
        insertion_mode: CorpusInsertionMode,
        dedupe_by_coverage: bool,
    ) -> Option<CampaignCorpusEntry> {
        let campaign_entry = matches!(insertion_mode, CorpusInsertionMode::Deferred)
            .then(|| CampaignCorpusEntry { tx_seq: corpus.tx_seq.clone(), dedupe_by_coverage });

        if matches!(insertion_mode, CorpusInsertionMode::Live)
            && let Some(worker_dir) = &self.worker_dir
        {
            let worker_corpus = worker_dir.join(CORPUS_DIR);
            let write_result = corpus.write_to_disk_in(&worker_corpus, self.config.corpus_gzip);
            if let Err(err) = write_result {
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
        campaign_entry
    }

    fn push_corpus_entry(&mut self, corpus: CorpusEntry) {
        let new_index = self.in_memory_corpus.len();
        self.new_entry_indices.push(new_index);
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
        Self::persist_campaign_outputs(&self.config, Vec::new(), optimization_best);
    }

    /// Persists logical-campaign corpus and optimization outputs after worker results have merged.
    pub(crate) fn persist_campaign_outputs(
        config: &FuzzCorpusConfig,
        entries: impl IntoIterator<Item = CampaignCorpusEntry>,
        optimization_best: Option<(I256, &[BasicTxDetails])>,
    ) {
        let mut output_dir_ready = false;
        for entry in entries {
            if !output_dir_ready {
                prepare_campaign_output_dir(config);
                output_dir_ready = true;
            }
            persist_campaign_entry(config, entry);
        }

        persist_optimization_output(config, optimization_best);
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

    /// Generates new call sequence from in memory corpus. Evicts oldest corpus mutated more than
    /// configured max mutations value. Used by invariant test campaigns.
    #[instrument(skip_all)]
    pub fn new_inputs(
        &mut self,
        test_runner: &mut TestRunner,
        fuzz_state: &InvariantFuzzState,
        targeted_contracts: &FuzzRunIdentifiedContracts,
    ) -> Result<Vec<BasicTxDetails>> {
        let mut new_seq = vec![];

        // Early return with first_input only if corpus dir / coverage guided fuzzing not
        // configured.
        if !self.config.is_coverage_guided() {
            new_seq.push(self.new_tx(test_runner)?);
            return Ok(new_seq);
        };

        if !self.in_memory_corpus.is_empty() {
            self.evict_oldest_corpus()?;

            let mutation_type =
                weighted_mutation_type(test_runner.rng(), &self.mutation_distribution);

            let rng = test_runner.rng();
            let corpus_len = self.in_memory_corpus.len();
            let primary_index = rng.random_range(0..corpus_len);
            let secondary_index = rng.random_range(0..corpus_len);
            let primary = &self.in_memory_corpus[primary_index];
            let secondary = &self.in_memory_corpus[secondary_index];

            match mutation_type {
                MutationType::Splice => {
                    trace!(target: "corpus", "splice {} and {}", primary.uuid, secondary.uuid);

                    self.current_mutated_index = Some(primary_index);

                    let start1 = rng.random_range(0..primary.tx_seq.len());
                    let end1 = rng.random_range(start1..primary.tx_seq.len());

                    let start2 = rng.random_range(0..secondary.tx_seq.len());
                    let end2 = rng.random_range(start2..secondary.tx_seq.len());

                    new_seq.reserve((end1 - start1) + (end2 - start2));
                    new_seq.extend_from_slice(&primary.tx_seq[start1..end1]);
                    new_seq.extend_from_slice(&secondary.tx_seq[start2..end2]);
                }
                MutationType::Repeat => {
                    let (corpus_index, corpus) = if rng.random::<bool>() {
                        (primary_index, primary)
                    } else {
                        (secondary_index, secondary)
                    };
                    trace!(target: "corpus", "repeat {}", corpus.uuid);

                    self.current_mutated_index = Some(corpus_index);

                    new_seq = corpus.tx_seq.clone();
                    let start = rng.random_range(0..corpus.tx_seq.len());
                    let end = rng.random_range(start..corpus.tx_seq.len());
                    let item_idx = rng.random_range(0..corpus.tx_seq.len());
                    let repeated = new_seq[item_idx].clone();
                    for tx in &mut new_seq[start..end] {
                        *tx = repeated.clone();
                    }
                }
                MutationType::Interleave => {
                    trace!(target: "corpus", "interleave {} with {}", primary.uuid, secondary.uuid);

                    self.current_mutated_index = Some(primary_index);

                    new_seq.reserve(primary.tx_seq.len().min(secondary.tx_seq.len()));
                    for (tx1, tx2) in primary.tx_seq.iter().zip(secondary.tx_seq.iter()) {
                        // TODO: chunks?
                        let tx = if rng.random::<bool>() { tx1.clone() } else { tx2.clone() };
                        new_seq.push(tx);
                    }
                }
                MutationType::Prefix => {
                    let (corpus_index, corpus) = if rng.random::<bool>() {
                        (primary_index, primary)
                    } else {
                        (secondary_index, secondary)
                    };
                    trace!(target: "corpus", "overwrite prefix of {}", corpus.uuid);

                    self.current_mutated_index = Some(corpus_index);

                    new_seq = corpus.tx_seq.clone();
                    for i in 0..rng.random_range(0..=new_seq.len()) {
                        new_seq[i] = self.new_tx(test_runner)?;
                    }
                }
                MutationType::Suffix => {
                    let (corpus_index, corpus) = if rng.random::<bool>() {
                        (primary_index, primary)
                    } else {
                        (secondary_index, secondary)
                    };
                    trace!(target: "corpus", "overwrite suffix of {}", corpus.uuid);

                    self.current_mutated_index = Some(corpus_index);

                    new_seq = corpus.tx_seq.clone();
                    for i in new_seq.len() - rng.random_range(0..new_seq.len())..corpus.tx_seq.len()
                    {
                        new_seq[i] = self.new_tx(test_runner)?;
                    }
                }
                MutationType::Abi => {
                    let targets = targeted_contracts.targets();
                    let (corpus_index, corpus) = if rng.random::<bool>() {
                        (primary_index, primary)
                    } else {
                        (secondary_index, secondary)
                    };
                    trace!(target: "corpus", "ABI mutate args of {}", corpus.uuid);

                    self.current_mutated_index = Some(corpus_index);

                    new_seq = corpus.tx_seq.clone();

                    let idx = rng.random_range(0..new_seq.len());
                    let tx = new_seq.get_mut(idx).unwrap();
                    if let (_, Some(function)) = targets.fuzzed_artifacts(tx) {
                        // TODO: add call_value to call details and mutate it as well as sender some
                        // of the time.
                        if !function.inputs.is_empty() {
                            self.abi_mutate(tx, function, test_runner, fuzz_state)?;
                        }
                    }
                }
                MutationType::Cmp => {
                    let targets = targeted_contracts.targets();
                    let (corpus_index, corpus) = if rng.random::<bool>() {
                        (primary_index, primary)
                    } else {
                        (secondary_index, secondary)
                    };
                    trace!(target: "corpus", "cmp mutate args of {}", corpus.uuid);

                    self.current_mutated_index = Some(corpus_index);

                    new_seq = corpus.tx_seq.clone();
                    let mut mutated = false;
                    let fallback_idx = rng.random_range(0..new_seq.len());
                    let candidates = || {
                        corpus
                            .cmp_seq
                            .iter()
                            .enumerate()
                            .filter(|(_, cmp_values)| !cmp_values.is_empty())
                    };
                    let candidate_count = candidates().count();
                    if candidate_count != 0 {
                        let start = rng.random_range(0..candidate_count);
                        for (idx, cmp_values) in
                            candidates().cycle().skip(start).take(candidate_count)
                        {
                            let tx = new_seq.get_mut(idx).unwrap();
                            if let (_, Some(function)) = targets.fuzzed_artifacts(tx) {
                                mutated = Self::cmp_mutate(
                                    tx,
                                    function,
                                    cmp_values.as_slice(),
                                    test_runner,
                                )?;
                                if mutated {
                                    break;
                                }
                            }
                        }
                    }

                    if !mutated && self.mutation_weights.mutation_weight_abi > 0 {
                        let tx = new_seq.get_mut(fallback_idx).unwrap();
                        if let (_, Some(function)) = targets.fuzzed_artifacts(tx)
                            && !function.inputs.is_empty()
                        {
                            self.abi_mutate(tx, function, test_runner, fuzz_state)?;
                        }
                    }
                }
            }
        }

        // Make sure the new sequence contains at least one tx to start fuzzing from.
        if new_seq.is_empty() {
            new_seq.push(self.new_tx(test_runner)?);
        }
        trace!(target: "corpus", "new sequence of {} calls generated", new_seq.len());

        Ok(new_seq)
    }

    /// Generates a new input from the shared in memory corpus.  Evicts oldest corpus mutated more
    /// than configured max mutations value. Used by fuzz (stateless) test campaigns.
    #[instrument(skip_all)]
    pub fn new_input(
        &mut self,
        test_runner: &mut TestRunner,
        fuzz_state: &EvmFuzzState,
        function: &Function,
    ) -> Result<Bytes> {
        // Early return if not running with coverage guided fuzzing.
        if !self.config.is_coverage_guided() {
            return Ok(self.new_tx(test_runner)?.call_details.calldata);
        }

        self.evict_oldest_corpus()?;

        let fresh_weight = self.config.corpus_random_sequence_weight.min(100);
        let generate_fresh = self.in_memory_corpus.is_empty()
            || (fresh_weight > 0 && test_runner.rng().random_ratio(fresh_weight, 100));

        let tx = if generate_fresh {
            self.current_mutated_index = None;
            self.new_tx(test_runner)?
        } else {
            let corpus_index = test_runner.rng().random_range(0..self.in_memory_corpus.len());
            let corpus = &self.in_memory_corpus[corpus_index];
            self.current_mutated_index = Some(corpus_index);
            let mut tx = corpus.tx_seq.first().unwrap().clone();
            let cmp_values = corpus.cmp_seq.first().map_or(&[][..], Vec::as_slice);
            match weighted_arg_mutation(test_runner.rng(), self.arg_mutation_distribution.as_ref())
            {
                Some(true)
                    if !Self::cmp_mutate(&mut tx, function, cmp_values, test_runner)?
                        && self.mutation_weights.mutation_weight_abi > 0
                        && !function.inputs.is_empty() =>
                {
                    self.abi_mutate(&mut tx, function, test_runner, fuzz_state)?;
                }
                Some(true) => {}
                Some(false)
                    if self.mutation_weights.mutation_weight_abi > 0
                        && !function.inputs.is_empty() =>
                {
                    self.abi_mutate(&mut tx, function, test_runner, fuzz_state)?;
                }
                Some(false) if self.mutation_weights.mutation_weight_cmp > 0 => {
                    let _ = Self::cmp_mutate(&mut tx, function, cmp_values, test_runner)?;
                }
                None => {
                    // Stateless fuzz inputs cannot apply sequence-level mutation strategies.
                    self.current_mutated_index = None;
                    return Ok(self.new_tx(test_runner)?.call_details.calldata);
                }
                _ => {}
            }
            tx
        };

        Ok(tx.call_details.calldata)
    }

    /// Generates single call from corpus strategy.
    pub fn new_tx(&self, test_runner: &mut TestRunner) -> Result<BasicTxDetails> {
        Ok(self
            .tx_generator
            .new_tree(test_runner)
            .map_err(|_| eyre!("Could not generate case"))?
            .current())
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
    ) -> Option<CampaignCorpusEntry> {
        if !self.config.is_coverage_guided() || observed.is_empty() {
            return None;
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
            let _ = self.push_observed_sequence(seq, insertion_mode);
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
    ) -> Option<CampaignCorpusEntry> {
        if !self.config.is_coverage_guided() || tx_seq.is_empty() {
            return None;
        }

        let corpus = CorpusEntry::new(tx_seq);

        self.insert_corpus_entry(corpus, insertion_mode, false)
    }

    /// Returns the next call to be used in call sequence.
    /// If coverage guided fuzzing is not configured or if previous input was discarded then this is
    /// a new tx from strategy.
    /// If running with coverage guided fuzzing it returns a new call only when sequence
    /// does not have enough entries, or randomly. Otherwise, returns the next call from initial
    /// sequence.
    pub fn generate_next_input(
        &mut self,
        test_runner: &mut TestRunner,
        sequence: &[BasicTxDetails],
        discarded: bool,
        depth: usize,
    ) -> Result<BasicTxDetails> {
        // Early return with new input if corpus dir / coverage guided fuzzing not configured or if
        // call was discarded.
        if self.config.corpus_dir.is_none() || discarded {
            return self.new_tx(test_runner);
        }

        // When running with coverage guided fuzzing enabled then generate new sequence if initial
        // sequence's length is less than depth or randomly, to occasionally intermix new txs.
        let fresh_weight = self.config.corpus_random_sequence_weight.min(100);
        let generate_fresh = fresh_weight > 0 && test_runner.rng().random_ratio(fresh_weight, 100);
        if depth >= sequence.len() || generate_fresh {
            return self.new_tx(test_runner);
        }

        // Continue with the next call initial sequence.
        Ok(sequence[depth].clone())
    }

    /// Flush the oldest corpus mutated more than configured max mutations unless they are
    /// favored.
    fn evict_oldest_corpus(&mut self) -> Result<()> {
        if self.in_memory_corpus.len() > self.config.corpus_min_size.max(1)
            && let Some(index) = self.in_memory_corpus.iter().position(|corpus| {
                corpus.total_mutations > self.config.corpus_min_mutations && !corpus.is_favored
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

    /// Mutates calldata of provided tx by abi decoding current values and randomly selecting the
    /// inputs to change.
    fn abi_mutate(
        &self,
        tx: &mut BasicTxDetails,
        function: &Function,
        test_runner: &mut TestRunner,
        fuzz_state: &impl FuzzStateReader,
    ) -> Result<()> {
        // Mutate value with configured probability for payable functions.
        if function.state_mutability == alloy_json_abi::StateMutability::Payable
            && test_runner.rng().random_ratio(self.config.payable_value_weight.min(100), 100)
        {
            tx.call_details.value = Some(generate_msg_value(test_runner));
        }

        // Mutate calldata.
        let mut arg_mutation_rounds =
            test_runner.rng().random_range(0..=function.inputs.len()).max(1);
        let round_arg_idx: Vec<usize> = if function.inputs.len() <= 1 {
            vec![0]
        } else {
            (0..arg_mutation_rounds)
                .map(|_| test_runner.rng().random_range(0..function.inputs.len()))
                .collect()
        };
        let mut prev_inputs = function
            .abi_decode_input(&tx.call_details.calldata[4..])
            .map_err(|err| eyre!("failed to load previous inputs: {err}"))?;

        while arg_mutation_rounds > 0 {
            let idx = round_arg_idx[arg_mutation_rounds - 1];
            prev_inputs[idx] = mutate_param_value(
                &function
                    .inputs
                    .get(idx)
                    .expect("Could not get input to mutate")
                    .selector_type()
                    .parse()?,
                prev_inputs[idx].clone(),
                test_runner,
                fuzz_state,
            );
            arg_mutation_rounds -= 1;
        }

        tx.call_details.calldata =
            function.abi_encode_input(&prev_inputs).map_err(|e| eyre!(e.to_string()))?.into();
        Ok(())
    }

    /// Mutates calldata by replacing bytes matching one side of an observed EVM comparison with
    /// the other side, following LibAFL's input-to-state replacement strategy.
    fn cmp_mutate(
        tx: &mut BasicTxDetails,
        function: &Function,
        cmp_values: &[CmpOperands],
        test_runner: &mut TestRunner,
    ) -> Result<bool> {
        if cmp_values.is_empty() || tx.call_details.calldata.len() <= 4 {
            return Ok(false);
        }

        let start = test_runner.rng().random_range(0..cmp_values.len());
        for offset in 0..cmp_values.len() {
            let cmp = &cmp_values[(start + offset) % cmp_values.len()];
            if let Some(mutated) =
                Self::cmp_mutated_calldata(tx.call_details.calldata.as_ref(), cmp, test_runner)
                && function.abi_decode_input(&mutated[4..]).is_ok()
            {
                tx.call_details.calldata = mutated.into();
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn cmp_mutated_calldata(
        calldata: &[u8],
        cmp: &CmpOperands,
        test_runner: &mut TestRunner,
    ) -> Option<Vec<u8>> {
        const WIDTHS: [usize; 6] = [32, 16, 8, 4, 2, 1];

        let lhs_full = cmp.op1.to_be_bytes::<32>();
        let rhs_full = cmp.op2.to_be_bytes::<32>();
        let width_start = test_runner.rng().random_range(0..WIDTHS.len());
        for offset in 0..WIDTHS.len() {
            let width = WIDTHS[(width_start + offset) % WIDTHS.len()];
            let lhs = &lhs_full[32 - width..];
            let rhs = &rhs_full[32 - width..];
            if lhs == rhs {
                continue;
            }

            let lhs_first = test_runner.rng().random::<bool>();
            let first = if lhs_first { (lhs, rhs) } else { (rhs, lhs) };
            let second = if lhs_first { (rhs, lhs) } else { (lhs, rhs) };

            if let Some(mutated) =
                Self::replace_cmp_operand(calldata, first.0, first.1, test_runner).or_else(|| {
                    Self::replace_cmp_operand(calldata, second.0, second.1, test_runner)
                })
            {
                return Some(mutated);
            }
        }

        None
    }

    fn replace_cmp_operand(
        calldata: &[u8],
        pattern: &[u8],
        replacement: &[u8],
        test_runner: &mut TestRunner,
    ) -> Option<Vec<u8>> {
        const SELECTOR_LEN: usize = 4;

        if pattern.is_empty()
            || pattern.len() != replacement.len()
            || calldata.len() < SELECTOR_LEN + pattern.len()
            || (pattern.len() < 32 && pattern.iter().all(|&b| b == 0))
        {
            return None;
        }

        let search_len = calldata.len() - SELECTOR_LEN - pattern.len() + 1;
        let start = test_runner.rng().random_range(0..search_len);
        for offset in 0..search_len {
            let idx = SELECTOR_LEN + ((start + offset) % search_len);
            if &calldata[idx..idx + pattern.len()] == pattern {
                let mut mutated = calldata.to_vec();
                mutated[idx..idx + replacement.len()].copy_from_slice(replacement);
                return Some(mutated);
            }
        }

        None
    }

    // Sync Methods.

    /// Imports the new corpus entries from the `sync` directory.
    /// These contain tx sequences which are replayed and used to update the history map.
    fn load_sync_corpus(&self) -> Result<Vec<(CorpusDirEntry, Vec<BasicTxDetails>)>> {
        let Some(worker_dir) = &self.worker_dir else {
            return Ok(vec![]);
        };

        let sync_dir = worker_dir.join(SYNC_DIR);
        if !sync_dir.is_dir() {
            return Ok(vec![]);
        }

        let mut imports = vec![];
        for entry in read_corpus_dir(&sync_dir) {
            if entry.timestamp <= self.last_sync_timestamp {
                continue;
            }
            // A corrupt or truncated sync file must not abort the whole sync pass: skip it.
            let tx_seq = match entry.read_tx_seq() {
                Ok(tx_seq) => tx_seq,
                Err(err) => {
                    warn!(target: "corpus", "skipping unreadable corpus file {}: {err}", entry.path.display());
                    continue;
                }
            };
            if tx_seq.is_empty() {
                warn!(target: "corpus", "skipping empty corpus entry: {}", entry.path.display());
                continue;
            }
            imports.push((entry, tx_seq));
        }

        if !imports.is_empty() {
            debug!(target: "corpus", "imported {} new corpus entries", imports.len());
        }

        Ok(imports)
    }

    /// Syncs and calibrates the in memory corpus and updates the history_map if new coverage is
    /// found from the corpus findings of other workers.
    #[instrument(skip_all)]
    fn calibrate<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        fuzzed_function: Option<&Function>,
        fuzzed_contracts: Option<&FuzzRunIdentifiedContracts>,
        dynamic: Option<&DynamicTargetCtx<'_>>,
    ) -> Result<()> {
        let Some(worker_dir) = &self.worker_dir else {
            return Ok(());
        };
        let corpus_dir = worker_dir.join(CORPUS_DIR);

        let mut executor = executor.clone();
        for (entry, tx_seq) in self.load_sync_corpus()? {
            let target = ReplayTarget { fuzzed_function, fuzzed_contracts, dynamic };
            let coverage = ReplayCoverage {
                history_map: &mut self.history_map,
                edge_indices: &mut self.edge_indices,
                sancov_history_map: &mut self.sancov_history_map,
                metrics: Some(&mut self.metrics),
            };
            let ReplayOutcome { keep_entry, new_coverage, new_edge, cmp_seq, .. } =
                replay_corpus_sequence_with_executor(
                    &tx_seq,
                    &mut executor,
                    target,
                    coverage,
                    true,
                    false,
                )?;

            // A synced edge is new to this worker's local map, so it advances the timer.
            if new_edge {
                self.last_new_edge_at = Some(Instant::now());
            }

            let sync_path = &entry.path;
            if keep_entry && new_coverage {
                // Move file from sync/ to corpus/ directory.
                let corpus_path = corpus_dir.join(sync_path.components().next_back().unwrap());
                if let Err(err) = std::fs::rename(sync_path, &corpus_path) {
                    debug!(target: "corpus", %err, "failed to move synced corpus from {sync_path:?} to {corpus_path:?} dir");
                    continue;
                }

                debug!(
                    target: "corpus",
                    name=%entry.name(),
                    "moved synced corpus to corpus dir",
                );

                let corpus_entry = CorpusEntry::new_with_cmp(tx_seq.clone(), cmp_seq, entry.uuid);
                self.in_memory_corpus.push(corpus_entry);
            } else {
                // Remove the file as it did not generate new coverage.
                if let Err(err) = std::fs::remove_file(&entry.path) {
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
    fn export_to_master(&self) -> Result<()> {
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

        for &index in &self.new_entry_indices {
            let Some(corpus) = self.in_memory_corpus.get(index) else { continue };
            let file_name = corpus.file_name(self.config.corpus_gzip);
            let file_path = corpus_dir.join(&file_name);
            let sync_path = master_sync_dir.join(&file_name);
            if let Err(err) = std::fs::hard_link(&file_path, &sync_path) {
                debug!(target: "corpus", %err, "failed to export corpus {}", corpus.uuid);
                continue;
            }
            exported += 1;
        }

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
        let filtered_master_corpus = read_corpus_dir(&master_corpus_dir)
            .filter(|entry| entry.timestamp > self.last_sync_timestamp)
            .collect::<Vec<_>>();
        let mut any_distributed = false;
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

            for entry in &filtered_master_corpus {
                let name = entry.name();
                let sync_path = target_dir.join(name);
                if let Err(err) = std::fs::hard_link(&entry.path, &sync_path) {
                    debug!(target: "corpus", %err, from=?entry.path, to=?sync_path, "failed to distribute corpus");
                    continue;
                }
                any_distributed = true;
                trace!(target: "corpus", %name, ?target_dir, "distributed corpus");
            }
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
        fuzzed_function: Option<&Function>,
        fuzzed_contracts: Option<&FuzzRunIdentifiedContracts>,
        dynamic: Option<&DynamicTargetCtx<'_>>,
        global_corpus_metrics: &GlobalCorpusMetrics,
    ) -> Result<()> {
        trace!(target: "corpus", "syncing");

        self.sync_metrics(global_corpus_metrics);

        self.calibrate(executor, fuzzed_function, fuzzed_contracts, dynamic)?;
        if self.id == 0 {
            self.export_to_workers(num_workers)?;
        } else {
            self.export_to_master()?;
        }

        let last_sync = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        self.last_sync_timestamp = last_sync;

        self.new_entry_indices.clear();

        debug!(target: "corpus", last_sync, "synced");

        Ok(())
    }

    /// Helper to check if a tx can be replayed.
    pub(crate) fn can_replay_tx(
        tx: &BasicTxDetails,
        fuzzed_function: Option<&Function>,
        fuzzed_contracts: Option<&FuzzRunIdentifiedContracts>,
    ) -> bool {
        fuzzed_contracts.is_some_and(|contracts| contracts.targets().can_replay(tx))
            || fuzzed_function.is_some_and(|function| {
                tx.call_details
                    .calldata
                    .get(..4)
                    .is_some_and(|selector| function.selector() == selector)
            })
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

fn prepare_campaign_output_dir(config: &FuzzCorpusConfig) {
    let Some(root) = &config.corpus_dir else {
        return;
    };
    let corpus_dir = root.join(format!("{WORKER}0")).join(CORPUS_DIR);
    if let Err(err) = foundry_common::fs::create_dir_all(&corpus_dir) {
        debug!(target: "corpus", %err, "failed to create campaign corpus dir");
    }
}

fn persist_campaign_entry(config: &FuzzCorpusConfig, entry: CampaignCorpusEntry) {
    let Some(root) = &config.corpus_dir else {
        return;
    };
    let corpus_dir = root.join(format!("{WORKER}0")).join(CORPUS_DIR);
    let corpus = CorpusEntry::new(entry.tx_seq);
    let write_result = corpus.write_to_disk_in(&corpus_dir, config.corpus_gzip);
    if let Err(err) = write_result {
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
    use crate::inspectors::{EdgeCovHit, EdgeCoverage, EdgeKey};
    use alloy_dyn_abi::DynSolValue;
    use foundry_config::FuzzDictionaryConfig;
    use proptest::prelude::Just;
    use revm::database::{CacheDB, EmptyDB};
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

    fn worker_corpus(id: usize, corpus_root: PathBuf, seed: WorkerCorpusSeed) -> WorkerCorpus {
        WorkerCorpus::from_seed(id, corpus_config(corpus_root), Just(basic_tx()).boxed(), seed)
            .unwrap()
    }

    fn worker_corpus_with_config(
        id: usize,
        config: FuzzCorpusConfig,
        generated: BasicTxDetails,
        seed: WorkerCorpusSeed,
    ) -> WorkerCorpus {
        WorkerCorpus::from_seed(id, config, Just(generated).boxed(), seed).unwrap()
    }

    fn empty_worker_corpus(id: usize, corpus_root: PathBuf) -> WorkerCorpus {
        worker_corpus(id, corpus_root, WorkerCorpusSeed::default())
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

    #[test]
    fn cmp_mutate_replaces_matching_calldata_operand() {
        let function = Function::parse("testCmp(uint256)").unwrap();
        let original = U256::from(7u64);
        let replacement = U256::from(42u64);
        let calldata: Bytes =
            function.abi_encode_input(&[DynSolValue::Uint(original, 256)]).unwrap().into();
        let mut tx = BasicTxDetails {
            warp: None,
            roll: None,
            sender: Address::ZERO,
            call_details: foundry_evm_fuzz::CallDetails {
                target: Address::ZERO,
                calldata,
                value: None,
            },
        };
        let cmp = CmpOperands {
            op1: original,
            op2: replacement,
            pc: 0,
            address: Address::ZERO,
            opcode: 0,
        };
        let config =
            proptest::test_runner::Config { failure_persistence: None, ..Default::default() };
        let mut runner = TestRunner::new(config);

        let mutated = WorkerCorpus::cmp_mutate(&mut tx, &function, &[cmp], &mut runner).unwrap();

        assert!(mutated);
        let decoded = function.abi_decode_input(&tx.call_details.calldata[4..]).unwrap();
        assert_eq!(decoded[0].as_uint().unwrap().0, replacement);
    }

    #[test]
    fn stateless_new_input_honors_fresh_sequence_weight() {
        let mut config = corpus_config(temp_corpus_dir());
        config.corpus_random_sequence_weight = 100;
        let generated = basic_tx_with_calldata(vec![0x22]);
        let seed = WorkerCorpusSeed {
            in_memory_corpus: vec![CorpusEntry::new(vec![basic_tx_with_calldata(vec![0x11])])],
            ..Default::default()
        };
        let mut manager = worker_corpus_with_config(0, config, generated, seed);
        let mut runner = TestRunner::default();
        let function = Function::parse("foo()").unwrap();

        let input = manager.new_input(&mut runner, &empty_fuzz_state(), &function).unwrap();

        assert_eq!(input, Bytes::from(vec![0x22]));
        assert!(manager.current_mutated_index.is_none());
    }

    #[test]
    fn stateless_new_input_does_not_fallback_to_disabled_arg_mutators() {
        let mut config = corpus_config(temp_corpus_dir());
        config.corpus_random_sequence_weight = 0;
        config.mutation_weights = FuzzCorpusMutationWeights {
            mutation_weight_splice: 1,
            mutation_weight_repeat: 1,
            mutation_weight_interleave: 1,
            mutation_weight_prefix: 1,
            mutation_weight_suffix: 1,
            mutation_weight_abi: 0,
            mutation_weight_cmp: 0,
        };
        let generated = basic_tx_with_calldata(vec![0x44]);
        let seed = WorkerCorpusSeed {
            in_memory_corpus: vec![CorpusEntry::new(vec![basic_tx_with_calldata(vec![0x33])])],
            ..Default::default()
        };
        let mut manager = worker_corpus_with_config(0, config, generated, seed);
        let mut runner = TestRunner::default();
        let function = Function::parse("foo(uint256)").unwrap();

        let input = manager.new_input(&mut runner, &empty_fuzz_state(), &function).unwrap();

        assert_eq!(input, Bytes::from(vec![0x44]));
        assert!(manager.current_mutated_index.is_none());
    }

    #[test]
    fn generate_next_input_handles_empty_sequence_with_fresh_weight_disabled() {
        let mut config = corpus_config(temp_corpus_dir());
        config.corpus_random_sequence_weight = 0;
        let generated = basic_tx_with_calldata(vec![0x55]);
        let mut manager =
            worker_corpus_with_config(0, config, generated.clone(), WorkerCorpusSeed::default());
        let mut runner = TestRunner::default();

        let input = manager.generate_next_input(&mut runner, &[], false, 0).unwrap();

        assert_eq!(input.call_details.calldata, generated.call_details.calldata);
    }

    #[test]
    fn mutation_weights_reject_overflowing_total() {
        let mut config = corpus_config(temp_corpus_dir());
        config.mutation_weights = FuzzCorpusMutationWeights {
            mutation_weight_splice: u32::MAX,
            mutation_weight_repeat: 1,
            mutation_weight_interleave: 0,
            mutation_weight_prefix: 0,
            mutation_weight_suffix: 0,
            mutation_weight_abi: 0,
            mutation_weight_cmp: 0,
        };

        let err = WorkerCorpus::from_seed(
            0,
            config,
            Just(basic_tx()).boxed(),
            WorkerCorpusSeed::default(),
        )
        .err()
        .unwrap();

        assert!(err.to_string().contains("effective mutation weights sum"));
    }

    #[test]
    fn invariant_cmp_mutation_does_not_fallback_to_disabled_abi_mutation() {
        let target = Address::from([0x42; 20]);
        let function = Function::parse("foo(uint256)").unwrap();
        let original = tx_for_function(target, &function, &[DynSolValue::Uint(U256::from(7), 256)]);
        let seed = WorkerCorpusSeed {
            in_memory_corpus: vec![CorpusEntry::new(vec![original.clone()])],
            ..Default::default()
        };
        let mut config = corpus_config(temp_corpus_dir());
        config.mutation_weights = FuzzCorpusMutationWeights {
            mutation_weight_splice: 0,
            mutation_weight_repeat: 0,
            mutation_weight_interleave: 0,
            mutation_weight_prefix: 0,
            mutation_weight_suffix: 0,
            mutation_weight_abi: 0,
            mutation_weight_cmp: 1,
        };
        let mut manager = worker_corpus_with_config(0, config, basic_tx(), seed);
        let mut runner = TestRunner::default();
        let fuzz_state = empty_fuzz_state().into_invariant();
        let targeted_contracts = targeted_contracts_with_selective_functions(
            target,
            vec![function.clone()],
            [function.selector()],
        );

        let sequence = manager.new_inputs(&mut runner, &fuzz_state, &targeted_contracts).unwrap();

        assert_eq!(sequence.len(), 1);
        assert_eq!(sequence[0].call_details.calldata, original.call_details.calldata);
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

    // A corrupt/truncated corpus file (valid name, unparseable content — e.g. a process killed
    // mid-write, since entries are persisted non-atomically) must surface as a per-entry read
    // error rather than break directory scanning, so the load/sync loops can skip it instead of
    // aborting the whole campaign.
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
    fn campaign_processing_returns_corpus_without_writing_worker_file() {
        let corpus_root = temp_corpus_dir();
        let worker_subdir = corpus_root.join("worker1");
        let mut manager = empty_worker_corpus(1, corpus_root);

        let record = manager.process_inputs_for_campaign(&[basic_tx()], &[], true, None);

        let record = record.unwrap();
        assert!(record.dedupe_by_coverage);
        assert_eq!(manager.in_memory_corpus.len(), 1);
        assert_eq!(manager.metrics.corpus_count, 1);
        assert_eq!(read_corpus_dir(&worker_subdir.join(CORPUS_DIR)).count(), 0);
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

        let record = manager.process_inputs_for_campaign(&[], &[], true, None);

        assert!(record.is_none());
        assert_eq!(manager.in_memory_corpus.len(), 0);
        assert_eq!(manager.metrics.corpus_count, 0);
        assert_eq!(read_corpus_dir(&worker_subdir.join(CORPUS_DIR)).count(), 0);

        // Live processing path must also tolerate the empty sequence.
        manager.process_inputs(&[], &[], true, None);
        assert_eq!(manager.in_memory_corpus.len(), 0);
        assert_eq!(read_corpus_dir(&worker_subdir.join(CORPUS_DIR)).count(), 0);
    }

    #[test]
    fn merged_campaign_outputs_write_corpus_and_optimization_to_master_dir() {
        let corpus_root = temp_corpus_dir();
        let mut manager = empty_worker_corpus(1, corpus_root.clone());
        let sequence = vec![basic_tx()];
        let record = manager
            .process_inputs_for_campaign(
                &sequence,
                &[],
                false,
                Some((I256::try_from(7).unwrap(), sequence.clone())),
            )
            .unwrap();
        let inputs = vec![record];
        WorkerCorpus::persist_campaign_outputs(
            &corpus_config(corpus_root.clone()),
            inputs,
            Some((I256::try_from(7).unwrap(), &sequence)),
        );

        let master_corpus_dir = corpus_root.join("worker0").join(CORPUS_DIR);
        let entries = read_corpus_dir(&master_corpus_dir).collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        let persisted_sequence = entries[0].read_tx_seq().unwrap();
        assert_eq!(persisted_sequence.len(), sequence.len());
        assert_eq!(persisted_sequence[0].sender, sequence[0].sender);
        assert_eq!(persisted_sequence[0].call_details.target, sequence[0].call_details.target);
        assert_eq!(persisted_sequence[0].call_details.calldata, sequence[0].call_details.calldata);

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
        let mut manager = WorkerCorpus::new::<foundry_evm_core::evm::EthEvmNetwork>(
            1,
            corpus_config(corpus_root),
            Just(basic_tx()).boxed(),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let worse_sequence = vec![basic_tx()];
        let worse = manager.process_inputs_for_campaign(
            &worse_sequence,
            &[],
            false,
            Some((I256::try_from(50).unwrap(), worse_sequence.clone())),
        );
        assert!(worse.is_none());

        let better_sequence = vec![basic_tx()];
        let better = manager.process_inputs_for_campaign(
            &better_sequence,
            &[],
            false,
            Some((I256::try_from(150).unwrap(), better_sequence.clone())),
        );
        assert!(better.is_some());
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
            failed_replays: 13,
            optimization_best_value: Some(I256::try_from(17).unwrap()),
            optimization_best_sequence: tx_seq,
            last_new_edge_at: None,
        };

        let manager =
            WorkerCorpus::from_seed(1, corpus_config(corpus_root), Just(basic_tx()).boxed(), seed)
                .unwrap();

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
            .map(|_| CorpusEntry::new_with_cmp(vec![basic_tx()], vec![vec![cmp]], Uuid::new_v4()))
            .collect::<Vec<_>>();
        let seed = WorkerCorpusSeed { in_memory_corpus: entries, ..Default::default() };

        let with_cmp = seed.clone_for_worker(0, 1, true);
        let without_cmp = seed.clone_for_worker(0, 1, false);

        assert!(with_cmp.in_memory_corpus.iter().all(|entry| !entry.cmp_seq[0].is_empty()));
        assert!(without_cmp.in_memory_corpus.iter().all(|entry| entry.cmp_seq.is_empty()));
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

        let campaign_entry = manager.hoist_observed_calls(
            &observed,
            &parent_tx,
            &targeted_contracts,
            CorpusInsertionMode::Live,
        );

        assert!(campaign_entry.is_none());
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
    fn hoist_observed_calls_returns_deferred_campaign_entry_without_persisting() {
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

        let campaign_entry = manager.hoist_observed_calls(
            &observed,
            &basic_tx(),
            &targeted_contracts,
            CorpusInsertionMode::Deferred,
        );

        let campaign_entry = campaign_entry.expect("deferred hoist should return campaign entry");
        assert!(!campaign_entry.dedupe_by_coverage);
        assert_eq!(campaign_entry.tx_seq.len(), 1);
        assert_eq!(manager.in_memory_corpus.len(), 1);
        assert_eq!(read_corpus_dir(&worker_corpus_dir).count(), 0);
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
        let mut manager = WorkerCorpus::from_seed(
            0,
            no_corpus_config,
            Just(basic_tx()).boxed(),
            WorkerCorpusSeed::default(),
        )
        .unwrap();
        assert!(
            manager
                .hoist_observed_calls(
                    &observed,
                    &basic_tx(),
                    &targeted_contracts,
                    CorpusInsertionMode::Live
                )
                .is_none()
        );
        assert!(manager.in_memory_corpus.is_empty());

        let mut manager = empty_worker_corpus(0, temp_corpus_dir());
        assert!(
            manager
                .hoist_observed_calls(
                    &[],
                    &basic_tx(),
                    &targeted_contracts,
                    CorpusInsertionMode::Live
                )
                .is_none()
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

        assert!(
            manager.push_observed_sequence(vec![basic_tx()], CorpusInsertionMode::Live).is_none()
        );
        assert_eq!(manager.in_memory_corpus.len(), 1);
        assert_eq!(read_corpus_dir(&worker0_corpus_dir).count(), 1);

        let mut manager = empty_worker_corpus(1, corpus_root.clone());
        let worker1_corpus_dir = corpus_root.join("worker1").join(CORPUS_DIR);
        assert!(
            manager
                .push_observed_sequence(vec![basic_tx()], CorpusInsertionMode::MemoryOnly)
                .is_none()
        );
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
}
