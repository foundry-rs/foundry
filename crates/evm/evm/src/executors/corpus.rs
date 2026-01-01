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

use crate::executors::{Executor, RawCallResult, invariant::execute_tx};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::Bytes;
use eyre::{Result, eyre};
use foundry_config::FuzzCorpusConfig;
use foundry_evm_fuzz::{
    BasicTxDetails,
    invariant::FuzzRunIdentifiedContracts,
    strategies::{EvmFuzzState, mutate_param_value},
};
use proptest::{
    prelude::{Just, Rng, Strategy},
    prop_oneof,
    strategy::{BoxedStrategy, ValueTree},
    test_runner::TestRunner,
};
use serde::Serialize;
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const WORKER: &str = "worker";
const CORPUS_DIR: &str = "corpus";
const SYNC_DIR: &str = "sync";

const COVERAGE_MAP_SIZE: usize = 65536;

/// Threshold for compressing corpus entries.
/// 4KiB is usually the minimum file size on popular file systems.
const GZIP_THRESHOLD: usize = 4 * 1024;

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
}

/// Holds Corpus information.
#[derive(Clone, Serialize)]
struct CorpusEntry {
    // Unique corpus identifier.
    uuid: Uuid,
    // Unique edge indices this entry hits.
    #[serde(skip_serializing)]
    unique_edges_covered: Vec<usize>,
    // Corpus call sequence.
    #[serde(skip_serializing)]
    tx_seq: Vec<BasicTxDetails>,
    // Whether this corpus is favored (part of minimal covering set).
    is_favored: bool,
    /// Timestamp of when this entry was written to disk in seconds.
    #[serde(skip_serializing)]
    timestamp: u64,
}

impl CorpusEntry {
    /// Creates a corpus entry with a new UUID.
    pub fn new(tx_seq: Vec<BasicTxDetails>) -> Self {
        Self::new_with(tx_seq, Uuid::new_v4())
    }

    /// Creates a corpus entry with a path.
    /// The UUID is parsed from the file name, otherwise a new UUID is generated.
    pub fn new_existing(tx_seq: Vec<BasicTxDetails>, path: PathBuf) -> Result<Self> {
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            eyre::bail!("invalid corpus file path: {path:?}");
        };
        let uuid = parse_corpus_filename(name)?.0;
        Ok(Self::new_with(tx_seq, uuid))
    }

    /// Creates a corpus entry with the given UUID.
    pub fn new_with(tx_seq: Vec<BasicTxDetails>, uuid: Uuid) -> Self {
        Self {
            uuid,
            unique_edges_covered: Vec::new(),
            tx_seq,
            is_favored: false,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time went backwards")
                .as_secs(),
        }
    }

    /// Creates a corpus entry with edges covered.
    pub fn with_edges(tx_seq: Vec<BasicTxDetails>, edges_covered: Vec<usize>) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            unique_edges_covered: edges_covered,
            tx_seq,
            is_favored: false,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time went backwards")
                .as_secs(),
        }
    }

    // TODO HashSet?

    pub fn set_edges(&mut self, mut edges_covered: Vec<usize>) {
        edges_covered.sort_unstable();
        edges_covered.dedup();
        self.unique_edges_covered = edges_covered;
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
        writeln!(f, "        - cumulative edges seen: {}", self.cumulative_edges_seen)?;
        writeln!(f, "        - cumulative features seen: {}", self.cumulative_features_seen)?;
        writeln!(f, "        - corpus count: {}", self.corpus_count)?;
        write!(f, "        - favored items: {}", self.favored_items)?;
        Ok(())
    }
}

impl CorpusMetrics {
    /// Records number of new edges or features explored during the campaign.
    pub fn update_seen(&mut self, is_edge: bool) {
        if is_edge {
            self.cumulative_edges_seen += 1;
        } else {
            self.cumulative_features_seen += 1;
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
    /// Tracks the best corpus for each edge
    top_rated: Vec<Option<(Uuid, usize)>>,
    /// Number of failed replays from initial corpus
    pub(crate) failed_replays: usize,
    /// Worker Metrics
    pub(crate) metrics: CorpusMetrics,
    /// Fuzzed calls generator.
    tx_generator: BoxedStrategy<BasicTxDetails>,
    /// Call sequence mutation strategy type generator used by stateful fuzzing.
    mutation_generator: BoxedStrategy<MutationType>,
    /// Identifier of current mutated entry for this worker.
    current_mutated: Option<Uuid>,
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
}

impl WorkerCorpus {
    pub fn new(
        id: usize,
        config: FuzzCorpusConfig,
        tx_generator: BoxedStrategy<BasicTxDetails>,
        // Only required by master worker (id = 0) to replay existing corpus.
        executor: Option<&Executor>,
        fuzzed_function: Option<&Function>,
        fuzzed_contracts: Option<&FuzzRunIdentifiedContracts>,
    ) -> Result<Self> {
        let mutation_generator = prop_oneof![
            Just(MutationType::Splice),
            Just(MutationType::Repeat),
            Just(MutationType::Interleave),
            Just(MutationType::Prefix),
            Just(MutationType::Suffix),
            Just(MutationType::Abi),
        ]
        .boxed();

        let worker_dir = config.corpus_dir.as_ref().map(|corpus_dir| {
            let worker_dir = corpus_dir.join(format!("{WORKER}{id}"));
            let worker_corpus = worker_dir.join(CORPUS_DIR);
            let sync_dir = worker_dir.join(SYNC_DIR);

            // Create the necessary directories for the worker.
            let _ = foundry_common::fs::create_dir_all(&worker_corpus);
            let _ = foundry_common::fs::create_dir_all(&sync_dir);

            worker_dir
        });

        let mut in_memory_corpus: Vec<CorpusEntry> = vec![];
        let mut history_map = vec![0u8; COVERAGE_MAP_SIZE];
        let mut top_rated = vec![None; COVERAGE_MAP_SIZE];
        let mut metrics = CorpusMetrics::default();
        let mut failed_replays = 0;

        let mut worker = Self {
            id,
            in_memory_corpus,
            history_map,
            top_rated,
            failed_replays,
            metrics,
            tx_generator,
            mutation_generator,
            current_mutated: None,
            config: config.into(),
            new_entry_indices: Default::default(),
            last_sync_timestamp: 0,
            worker_dir,
            last_sync_metrics: Default::default(),
        };

        if id == 0
            && let Some(corpus_dir) = &worker.config.corpus_dir
        {
            // Master worker loads the initial corpus, if it exists.
            // Then, [distribute]s it to workers.
            let executor = executor.expect("Executor required for master worker");
            'corpus_replay: for entry in read_corpus_dir(corpus_dir) {
                let tx_seq = entry.read_tx_seq()?;
                if tx_seq.is_empty() {
                    continue;
                }
                // Warm up history map from loaded sequences.
                let mut executor = executor.clone();
                let mut cumulative_edges = Vec::new();
                for tx in &tx_seq {
                    if Self::can_replay_tx(tx, fuzzed_function, fuzzed_contracts) {
                        let mut call_result = execute_tx(&mut executor, tx)?;
                        let (new_coverage, is_edge, edges) =
                            call_result.merge_edge_coverage_detailed(&mut worker.history_map);
                        cumulative_edges.extend(edges);
                        if new_coverage {
                            worker.metrics.update_seen(is_edge);
                        }

                        // Commit only when running invariant / stateful tests.
                        if fuzzed_contracts.is_some() {
                            executor.commit(&mut call_result);
                        }
                    } else {
                        failed_replays += 1;

                        // If the only input for fuzzed function cannot be replied, then move to
                        // next one without adding it in memory.
                        if fuzzed_function.is_some() {
                            continue 'corpus_replay;
                        }
                    }
                }

                worker.metrics.corpus_count += 1;

                debug!(
                    target: "corpus",
                    "load sequence with len {} from corpus file {}",
                    tx_seq.len(),
                    entry.path.display()
                );

                // Populate in memory corpus with the sequence from corpus file.
                let mut corpus_entry = CorpusEntry::new_with(tx_seq, entry.uuid);
                corpus_entry.set_edges(cumulative_edges);

                worker.update_top_rated(&corpus_entry);

                worker.in_memory_corpus.push(corpus_entry);
            }
        }

        // Compute favored set after loading corpus
        worker.recompute_favored_and_cull_corpus()?;

        Ok(worker)
    }

    /// Updates stats for the given call sequence, if new coverage produced.
    /// Persists the call sequence (if corpus directory is configured and new coverage) and updates
    /// in-memory corpus.
    #[instrument(skip_all)]
    pub fn process_inputs(
        &mut self,
        inputs: &[BasicTxDetails],
        new_coverage: bool,
        edges_covered: Vec<usize>,
    ) {
        let Some(worker_corpus) = &self.worker_dir else {
            return;
        };
        let worker_corpus = worker_corpus.join(CORPUS_DIR);

        // Clear current mutated tracking
        self.current_mutated = None;

        // Collect inputs only if current run produced new coverage.
        if !new_coverage {
            return;
        }

        assert!(!inputs.is_empty());
        let corpus = CorpusEntry::with_edges(inputs.to_vec(), edges_covered);

        // Update top_rated before adding to corpus
        self.update_top_rated(&corpus);

        // Persist to disk.
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

        // Track in-memory corpus changes to update MasterWorker on sync.
        let new_index = self.in_memory_corpus.len();
        self.new_entry_indices.push(new_index);

        // This includes reverting txs in the corpus and `can_continue` removes
        // them. We want this as it is new coverage and may help reach the other branch.
        self.metrics.corpus_count += 1;
        self.in_memory_corpus.push(corpus);
    }

    /// Collects coverage from call result and updates metrics.
    /// Returns (new_coverage_found, edges_hit)
    pub fn merge_edge_coverage(&mut self, call_result: &mut RawCallResult) -> (bool, Vec<usize>) {
        if !self.config.collect_edge_coverage() {
            return (false, Vec::new());
        }

        let (new_coverage, is_edge, edges) =
            call_result.merge_edge_coverage_detailed(&mut self.history_map);
        if new_coverage {
            self.metrics.update_seen(is_edge);
        }
        (new_coverage, edges)
    }

    /// Updates top_rated entries when a new corpus is added.
    /// If the new corpus covers an edge with lower cost than the current
    /// top_rated entry, it becomes the new top_rated for that edge.
    fn update_top_rated(&mut self, new_corpus: &CorpusEntry) {
        for &edge_idx in &new_corpus.unique_edges_covered {
            match &mut self.top_rated[edge_idx] {
                // No corpus covers this edge yet
                None => {
                    self.top_rated[edge_idx] = Some((new_corpus.uuid, new_corpus.tx_seq.len()));
                }
                // Check if new corpus is cheaper
                Some((best_uuid, best_cost)) => {
                    if new_corpus.tx_seq.len() < *best_cost {
                        *best_uuid = new_corpus.uuid;
                        *best_cost = new_corpus.tx_seq.len();
                    }
                }
            };
        }
    }

    /// Recomputes which corpus entries are favored.
    /// Uses greedy set cover: prioritize rare edges, pick the smallest
    /// corpus that covers each, mark it favored, repeat.
    fn recompute_favored_and_cull_corpus(&mut self) -> Result<()> {
        if self.in_memory_corpus.is_empty() {
            return Ok(());
        }

        // Step 1: Reset all favored flags
        for corpus in &mut self.in_memory_corpus {
            corpus.is_favored = false;
        }

        // Step 2: Build list of (edge_index, rarity) for edges we've seen
        // Rarity = how many corpus entries cover this edge
        let mut edge_rarity: Vec<(usize, usize)> = Vec::new();

        for edge_idx in 0..COVERAGE_MAP_SIZE {
            // Only consider edges that have a top_rated entry
            if self.top_rated[edge_idx].is_some() {
                let count = self
                    .in_memory_corpus
                    .iter()
                    .filter(|c| c.unique_edges_covered.contains(&edge_idx))
                    .count();

                if count > 0 {
                    edge_rarity.push((edge_idx, count));
                }
            }
        }

        // Step 3: Sort by rarity (rarest edges first)
        edge_rarity.sort_by_key(|&(_, count)| count);

        // Step 4: Track which edges we've covered
        let mut covered = vec![false; COVERAGE_MAP_SIZE];

        // Step 5: Greedy selection
        for (edge_idx, _) in edge_rarity {
            if covered[edge_idx] {
                continue; // Already covered by a favored corpus
            }

            // Get the top_rated corpus for this edge
            if let Some((uuid, _)) = self.top_rated[edge_idx] {
                if let Some(corpus) = self.in_memory_corpus.iter_mut().find(|c| c.uuid == uuid) {
                    // Mark this corpus as favored
                    corpus.is_favored = true;

                    // Mark all edges this corpus covers as covered
                    for &e in &corpus.unique_edges_covered {
                        covered[e] = true;
                    }
                }
            }
        }

        // Step 6: Update metrics
        self.metrics.favored_items = self.in_memory_corpus.iter().filter(|c| c.is_favored).count();

        self.cull_corpus()
    }

    /// Generates new call sequence from in memory corpus. Used by invariant test campaigns
    #[instrument(skip_all)]
    pub fn new_inputs(
        &mut self,
        test_runner: &mut TestRunner,
        fuzz_state: &EvmFuzzState,
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
            let mutation_type = self
                .mutation_generator
                .new_tree(test_runner)
                .map_err(|err| eyre!("Could not generate mutation type {err}"))?
                .current();

            let rng = test_runner.rng();
            let corpus_len = self.in_memory_corpus.len();
            let primary = &self.in_memory_corpus[rng.random_range(0..corpus_len)];
            let secondary = &self.in_memory_corpus[rng.random_range(0..corpus_len)];

            match mutation_type {
                MutationType::Splice => {
                    trace!(target: "corpus", "splice {} and {}", primary.uuid, secondary.uuid);

                    self.current_mutated = Some(primary.uuid);

                    let start1 = rng.random_range(0..primary.tx_seq.len());
                    let end1 = rng.random_range(start1..primary.tx_seq.len());

                    let start2 = rng.random_range(0..secondary.tx_seq.len());
                    let end2 = rng.random_range(start2..secondary.tx_seq.len());

                    for tx in primary.tx_seq.iter().take(end1).skip(start1) {
                        new_seq.push(tx.clone());
                    }
                    for tx in secondary.tx_seq.iter().take(end2).skip(start2) {
                        new_seq.push(tx.clone());
                    }
                }
                MutationType::Repeat => {
                    let corpus = if rng.random::<bool>() { primary } else { secondary };
                    trace!(target: "corpus", "repeat {}", corpus.uuid);

                    self.current_mutated = Some(corpus.uuid);

                    new_seq = corpus.tx_seq.clone();
                    let start = rng.random_range(0..corpus.tx_seq.len());
                    let end = rng.random_range(start..corpus.tx_seq.len());
                    let item_idx = rng.random_range(0..corpus.tx_seq.len());
                    let repeated = vec![new_seq[item_idx].clone(); end - start];
                    new_seq.splice(start..end, repeated);
                }
                MutationType::Interleave => {
                    trace!(target: "corpus", "interleave {} with {}", primary.uuid, secondary.uuid);

                    self.current_mutated = Some(primary.uuid);

                    for (tx1, tx2) in primary.tx_seq.iter().zip(secondary.tx_seq.iter()) {
                        // TODO: chunks?
                        let tx = if rng.random::<bool>() { tx1.clone() } else { tx2.clone() };
                        new_seq.push(tx);
                    }
                }
                MutationType::Prefix => {
                    let corpus = if rng.random::<bool>() { primary } else { secondary };
                    trace!(target: "corpus", "overwrite prefix of {}", corpus.uuid);

                    self.current_mutated = Some(corpus.uuid);

                    new_seq = corpus.tx_seq.clone();
                    for i in 0..rng.random_range(0..=new_seq.len()) {
                        new_seq[i] = self.new_tx(test_runner)?;
                    }
                }
                MutationType::Suffix => {
                    let corpus = if rng.random::<bool>() { primary } else { secondary };
                    trace!(target: "corpus", "overwrite suffix of {}", corpus.uuid);

                    self.current_mutated = Some(corpus.uuid);

                    new_seq = corpus.tx_seq.clone();
                    for i in new_seq.len() - rng.random_range(0..new_seq.len())..corpus.tx_seq.len()
                    {
                        new_seq[i] = self.new_tx(test_runner)?;
                    }
                }
                MutationType::Abi => {
                    let targets = targeted_contracts.targets.lock();
                    let corpus = if rng.random::<bool>() { primary } else { secondary };
                    trace!(target: "corpus", "ABI mutate args of {}", corpus.uuid);

                    self.current_mutated = Some(corpus.uuid);

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
            }
        }

        // Make sure the new sequence contains at least one tx to start fuzzing from.
        if new_seq.is_empty() {
            new_seq.push(self.new_tx(test_runner)?);
        }
        trace!(target: "corpus", "new sequence of {} calls generated", new_seq.len());

        Ok(new_seq)
    }

    /// Generates a new input from the shared in memory corpus. Used by fuzz (stateless) test
    /// campaigns.
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

        let tx = if !self.in_memory_corpus.is_empty() {
            let corpus = &self.in_memory_corpus
                [test_runner.rng().random_range(0..self.in_memory_corpus.len())];
            self.current_mutated = Some(corpus.uuid);
            let mut tx = corpus.tx_seq.first().unwrap().clone();
            self.abi_mutate(&mut tx, function, test_runner, fuzz_state)?;
            tx
        } else {
            self.new_tx(test_runner)?
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
        if depth > sequence.len().saturating_sub(1) || test_runner.rng().random_ratio(1, 10) {
            return self.new_tx(test_runner);
        }

        // Continue with the next call initial sequence.
        Ok(sequence[depth].clone())
    }

    /// Flush the non-favored corpus entries when the corpus size exceeds the minimum.
    fn cull_corpus(&mut self) -> Result<()> {
        if self.in_memory_corpus.len() > self.config.corpus_min_size.max(1)
            && let Some(index) = self.in_memory_corpus.iter().position(|corpus| !corpus.is_favored)
        {
            let corpus = &self.in_memory_corpus[index];
            let evicted_uuid = corpus.uuid;

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

            // Update top_rated entries that pointed to the evicted corpus
            for edge_idx in 0..COVERAGE_MAP_SIZE {
                if let Some((uuid, _)) = self.top_rated[edge_idx] {
                    if uuid == evicted_uuid {
                        // Find the next best corpus for this edge
                        self.top_rated[edge_idx] = self
                            .in_memory_corpus
                            .iter()
                            .filter(|c| c.unique_edges_covered.contains(&edge_idx))
                            .min_by_key(|c| c.tx_seq.len())
                            .map(|c| (c.uuid, c.tx_seq.len()));

                        // If we evicted a non-favored corpus, there must be another corpus
                        // covering this edge (otherwise the evicted corpus would be favored)
                        assert!(
                            self.top_rated[edge_idx].is_some(),
                            "evicted non-favored corpus was the only one covering edge {}",
                            edge_idx
                        );
                    }
                }
            }
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
        fuzz_state: &EvmFuzzState,
    ) -> Result<()> {
        // let rng = test_runner.rng();
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
            let tx_seq = entry.read_tx_seq()?;
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
    fn calibrate(
        &mut self,
        executor: &Executor,
        fuzzed_function: Option<&Function>,
        fuzzed_contracts: Option<&FuzzRunIdentifiedContracts>,
    ) -> Result<()> {
        let Some(worker_dir) = &self.worker_dir else {
            return Ok(());
        };
        let corpus_dir = worker_dir.join(CORPUS_DIR);

        let mut executor = executor.clone();
        for (entry, tx_seq) in self.load_sync_corpus()? {
            let mut new_coverage_on_sync = false;
            let mut corpus_edges = Vec::new();
            for tx in &tx_seq {
                if !Self::can_replay_tx(tx, fuzzed_function, fuzzed_contracts) {
                    continue;
                }

                let mut call_result = execute_tx(&mut executor, tx)?;

                // Check if this provides new coverage.
                let (new_coverage, is_edge, edges) =
                    call_result.merge_edge_coverage_detailed(&mut self.history_map);

                if new_coverage {
                    self.metrics.update_seen(is_edge);
                    new_coverage_on_sync = true;
                    corpus_edges.extend(edges);
                }

                // Commit only for stateful tests.
                if fuzzed_contracts.is_some() {
                    executor.commit(&mut call_result);
                }

                trace!(
                    target: "corpus",
                    %new_coverage,
                    ?tx,
                    "replayed tx for syncing",
                );
            }

            let sync_path = &entry.path;
            if new_coverage_on_sync {
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

                let mut corpus_entry =
                    CorpusEntry::new_existing(tx_seq.to_vec(), entry.path.clone())?;
                corpus_entry.unique_edges_covered = corpus_edges;

                // Update top_rated before adding
                self.update_top_rated(&corpus_entry);

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

        self.recompute_favored_and_cull_corpus()?;
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
    pub fn sync(
        &mut self,
        num_workers: usize,
        executor: &Executor,
        fuzzed_function: Option<&Function>,
        fuzzed_contracts: Option<&FuzzRunIdentifiedContracts>,
        global_corpus_metrics: &GlobalCorpusMetrics,
    ) -> Result<()> {
        trace!(target: "corpus", "syncing");

        self.sync_metrics(global_corpus_metrics);

        self.calibrate(executor, fuzzed_function, fuzzed_contracts)?;
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
    fn can_replay_tx(
        tx: &BasicTxDetails,
        fuzzed_function: Option<&Function>,
        fuzzed_contracts: Option<&FuzzRunIdentifiedContracts>,
    ) -> bool {
        fuzzed_contracts.is_some_and(|contracts| contracts.targets.lock().can_replay(tx))
            || fuzzed_function.is_some_and(|function| {
                tx.call_details
                    .calldata
                    .get(..4)
                    .is_some_and(|selector| function.selector() == selector)
            })
    }
}

fn read_corpus_dir(path: &Path) -> impl Iterator<Item = CorpusDirEntry> + use<> {
    let dir = match std::fs::read_dir(path) {
        Ok(dir) => dir,
        Err(err) => {
            debug!(%err, ?path, "failed to read corpus directory");
            return vec![].into_iter();
        }
    };
    dir.filter_map(|res| match res {
        Ok(entry) => {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let name = if path.is_file()
                && let Some(name) = path.file_name()
                && let Some(name) = name.to_str()
            {
                name
            } else {
                return None;
            };

            if let Ok((uuid, timestamp)) = parse_corpus_filename(name) {
                Some(CorpusDirEntry { path, uuid, timestamp })
            } else {
                debug!(target: "corpus", ?path, "failed to parse corpus filename");
                None
            }
        }
        Err(err) => {
            debug!(%err, "failed to read corpus directory entry");
            None
        }
    })
    .collect::<Vec<_>>()
    .into_iter()
}

struct CorpusDirEntry {
    path: PathBuf,
    uuid: Uuid,
    timestamp: u64,
}

impl CorpusDirEntry {
    fn name(&self) -> &str {
        self.path.file_name().unwrap().to_str().unwrap()
    }

    fn read_tx_seq(&self) -> foundry_common::fs::Result<Vec<BasicTxDetails>> {
        let path = &self.path;
        if path.extension() == Some("gz".as_ref()) {
            foundry_common::fs::read_json_gzip_file(path)
        } else {
            foundry_common::fs::read_json_file(path)
        }
    }
}

/// Parses the corpus filename and returns the uuid and timestamp associated with it.
fn parse_corpus_filename(name: &str) -> Result<(Uuid, u64)> {
    let name = name.trim_end_matches(".gz").trim_end_matches(".json");

    let (uuid_str, timestamp_str) =
        name.rsplit_once('-').ok_or_else(|| eyre!("invalid corpus filename format: {name}"))?;

    let uuid = Uuid::parse_str(uuid_str)?;
    let timestamp = timestamp_str.parse()?;

    Ok((uuid, timestamp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Address;
    use std::{fs, vec};

    fn basic_tx() -> BasicTxDetails {
        BasicTxDetails {
            warp: None,
            roll: None,
            sender: Address::ZERO,
            call_details: foundry_evm_fuzz::CallDetails {
                target: Address::ZERO,
                calldata: Bytes::new(),
            },
        }
    }

    fn temp_corpus_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("foundry-corpus-tests-{}", Uuid::new_v4()));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn new_manager_with_single_corpus() -> (WorkerCorpus, Uuid) {
        let tx_gen = Just(basic_tx()).boxed();
        let config = FuzzCorpusConfig {
            corpus_dir: Some(temp_corpus_dir()),
            corpus_gzip: false,
            corpus_min_mutations: 0,
            corpus_min_size: 0,
            ..Default::default()
        };

        let tx_seq = vec![basic_tx()];
        let corpus = CorpusEntry::new(tx_seq);
        let seed_uuid = corpus.uuid;

        // Create corpus root dir and worker subdirectory.
        let corpus_root = config.corpus_dir.clone().unwrap();
        let worker_subdir = corpus_root.join("worker0");
        let _ = fs::create_dir_all(&worker_subdir);

        let manager = WorkerCorpus {
            id: 0,
            tx_generator: tx_gen,
            mutation_generator: Just(MutationType::Repeat).boxed(),
            config: config.into(),
            in_memory_corpus: vec![corpus],
            current_mutated: Some(seed_uuid),
            failed_replays: 0,
            history_map: vec![0u8; COVERAGE_MAP_SIZE],
            top_rated: vec![None; COVERAGE_MAP_SIZE],
            metrics: CorpusMetrics::default(),
            new_entry_indices: Default::default(),
            last_sync_timestamp: 0,
            worker_dir: Some(corpus_root),
            last_sync_metrics: CorpusMetrics::default(),
        };

        (manager, seed_uuid)
    }

    #[test]
    fn top_rated_tracks_smallest_corpus_per_edge() {
        let tx_gen = Just(basic_tx()).boxed();
        let config = FuzzCorpusConfig {
            corpus_dir: Some(temp_corpus_dir()),
            corpus_gzip: false,
            corpus_min_mutations: 0,
            corpus_min_size: 0,
            ..Default::default()
        };

        // Create corpus A covering edges [0, 1, 2] with cost 5
        let corpus_a = CorpusEntry::with_edges(vec![basic_tx(); 5], vec![0, 1, 2]);
        let uuid_a = corpus_a.uuid;

        // Create corpus B covering edges [1, 2, 3] with cost 3
        let corpus_b = CorpusEntry::with_edges(vec![basic_tx(); 3], vec![1, 2, 3]);
        let uuid_b = corpus_b.uuid;

        let corpus_root = temp_corpus_dir();
        let _ = fs::create_dir_all(&corpus_root);

        let mut manager = WorkerCorpus {
            id: 0,
            tx_generator: tx_gen,
            mutation_generator: Just(MutationType::Repeat).boxed(),
            config: config.into(),
            in_memory_corpus: vec![],
            current_mutated: None,
            failed_replays: 0,
            history_map: vec![0u8; COVERAGE_MAP_SIZE],
            top_rated: vec![None; COVERAGE_MAP_SIZE],
            metrics: CorpusMetrics::default(),
            new_entry_indices: Default::default(),
            last_sync_timestamp: 0,
            worker_dir: Some(corpus_root),
            last_sync_metrics: CorpusMetrics::default(),
        };

        // Add corpus A
        manager.update_top_rated(&corpus_a);
        manager.in_memory_corpus.push(corpus_a);

        // Verify top_rated[0] = A, top_rated[1] = A, top_rated[2] = A
        assert_eq!(manager.top_rated[0], Some((uuid_a, 5)));
        assert_eq!(manager.top_rated[1], Some((uuid_a, 5)));
        assert_eq!(manager.top_rated[2], Some((uuid_a, 5)));

        // Add corpus B
        manager.update_top_rated(&corpus_b);
        manager.in_memory_corpus.push(corpus_b);

        // Verify top_rated[0] = A (only A covers it)
        // top_rated[1] = B (B is cheaper: cost 3 < 5)
        // top_rated[2] = B (B is cheaper: cost 3 < 5)
        // top_rated[3] = B (only B covers it)
        assert_eq!(manager.top_rated[0], Some((uuid_a, 5)));
        assert_eq!(manager.top_rated[1], Some((uuid_b, 3)));
        assert_eq!(manager.top_rated[2], Some((uuid_b, 3)));
        assert_eq!(manager.top_rated[3], Some((uuid_b, 3)));
    }

    #[test]
    fn cull_queue_selects_minimal_covering_set() {
        let tx_gen = Just(basic_tx()).boxed();
        let config = FuzzCorpusConfig {
            corpus_dir: Some(temp_corpus_dir()),
            corpus_gzip: false,
            corpus_min_mutations: 0,
            corpus_min_size: 0,
            ..Default::default()
        };

        // Create corpus A covering edges [0, 1] with cost 2
        let corpus_a = CorpusEntry::with_edges(vec![basic_tx(); 2], vec![0, 1]);
        let uuid_a = corpus_a.uuid;

        // Create corpus B covering edges [1, 2] with cost 2
        let corpus_b = CorpusEntry::with_edges(vec![basic_tx(); 2], vec![1, 2]);
        let uuid_b = corpus_b.uuid;

        // Create corpus C covering edges [0, 1, 2] with cost 5
        let corpus_c = CorpusEntry::with_edges(vec![basic_tx(); 5], vec![0, 1, 2]);
        let uuid_c = corpus_c.uuid;

        let corpus_root = temp_corpus_dir();
        let _ = fs::create_dir_all(&corpus_root);

        let mut manager = WorkerCorpus {
            id: 0,
            tx_generator: tx_gen,
            mutation_generator: Just(MutationType::Repeat).boxed(),
            config: config.into(),
            in_memory_corpus: vec![corpus_a, corpus_b, corpus_c],
            current_mutated: None,
            failed_replays: 0,
            history_map: vec![0u8; COVERAGE_MAP_SIZE],
            top_rated: vec![None; COVERAGE_MAP_SIZE],
            metrics: CorpusMetrics::default(),
            new_entry_indices: Default::default(),
            last_sync_timestamp: 0,
            worker_dir: Some(corpus_root),
            last_sync_metrics: CorpusMetrics::default(),
        };

        // Update top_rated for all corpus entries
        let corpus_entries: Vec<_> = manager.in_memory_corpus.clone();
        for corpus in &corpus_entries {
            manager.update_top_rated(corpus);
        }

        let _ = manager.recompute_favored_and_cull_corpus();

        // Verify A and B are favored, C is not (A+B cover everything cheaper)
        let corpus_a = manager.in_memory_corpus.iter().find(|c| c.uuid == uuid_a).unwrap();
        let corpus_b = manager.in_memory_corpus.iter().find(|c| c.uuid == uuid_b).unwrap();
        assert!(
            manager.in_memory_corpus.iter().find(|c| c.uuid == uuid_c).is_none(),
            "corpus C should be culled"
        );

        assert!(corpus_a.is_favored, "corpus A should be favored");
        assert!(corpus_b.is_favored, "corpus B should be favored");

        // Verify metrics
        assert_eq!(manager.metrics.favored_items, 2);
    }

    #[test]
    fn eviction_updates_top_rated() {
        let tx_gen = Just(basic_tx()).boxed();
        let config = FuzzCorpusConfig {
            corpus_dir: Some(temp_corpus_dir()),
            corpus_gzip: false,
            corpus_min_mutations: 0,
            corpus_min_size: 1,
            ..Default::default()
        };

        // Create corpus A covering edge [0] with cost 2
        let mut corpus_a = CorpusEntry::with_edges(vec![basic_tx(); 2], vec![0]);
        corpus_a.is_favored = false; // Make it non-favored so it can be evicted
        let uuid_a = corpus_a.uuid;

        // Create corpus B covering edge [0] with cost 3 (higher cost)
        let mut corpus_b = CorpusEntry::with_edges(vec![basic_tx(); 3], vec![0]);
        corpus_b.is_favored = true; // Make it favored so it won't be evicted
        let uuid_b = corpus_b.uuid;

        let corpus_root = temp_corpus_dir();
        let _ = fs::create_dir_all(&corpus_root);

        let mut manager = WorkerCorpus {
            id: 0,
            tx_generator: tx_gen,
            mutation_generator: Just(MutationType::Repeat).boxed(),
            config: config.into(),
            in_memory_corpus: vec![corpus_a, corpus_b],
            current_mutated: None,
            failed_replays: 0,
            history_map: vec![0u8; COVERAGE_MAP_SIZE],
            top_rated: vec![None; COVERAGE_MAP_SIZE],
            metrics: CorpusMetrics::default(),
            new_entry_indices: Default::default(),
            last_sync_timestamp: 0,
            worker_dir: Some(corpus_root),
            last_sync_metrics: CorpusMetrics::default(),
        };

        // Update top_rated - A should be top rated because it's cheaper
        let corpus_entries: Vec<_> = manager.in_memory_corpus.clone();
        for corpus in &corpus_entries {
            manager.update_top_rated(corpus);
        }

        // Verify top_rated[0] = A
        assert_eq!(manager.top_rated[0], Some((uuid_a, 2)));

        // Evict A (it's not favored)
        manager.cull_corpus().unwrap();

        // Verify A was removed
        assert_eq!(manager.in_memory_corpus.len(), 1);
        assert!(manager.in_memory_corpus.iter().all(|c| c.uuid != uuid_a));

        // Verify top_rated[0] = B (after A was evicted)
        assert_eq!(manager.top_rated[0], Some((uuid_b, 3)));
    }

    #[test]
    fn eviction_skips_favored_and_evicts_non_favored() {
        // Manager with two corpora.
        let tx_gen = Just(basic_tx()).boxed();
        let config = FuzzCorpusConfig {
            corpus_dir: Some(temp_corpus_dir()),
            corpus_min_mutations: 0,
            corpus_min_size: 0,
            ..Default::default()
        };

        // Create two corpus covering the same edge, with different costs
        // The cheaper one (cost=1) will be favored after recompute_favored_and_cull_corpus
        let mut favored = CorpusEntry::with_edges(vec![basic_tx()], vec![0]);
        favored.is_favored = false;
        let favored_uuid = favored.uuid;

        let mut non_favored = CorpusEntry::with_edges(vec![basic_tx(); 2], vec![0]);
        non_favored.is_favored = false;

        let corpus_root = temp_corpus_dir();
        let worker_subdir = corpus_root.join("worker0");
        fs::create_dir_all(&worker_subdir).unwrap();

        let mut manager = WorkerCorpus {
            id: 0,
            tx_generator: tx_gen,
            mutation_generator: Just(MutationType::Repeat).boxed(),
            config: config.into(),
            in_memory_corpus: vec![favored, non_favored],
            current_mutated: None,
            failed_replays: 0,
            history_map: vec![0u8; COVERAGE_MAP_SIZE],
            top_rated: vec![None; COVERAGE_MAP_SIZE],
            metrics: CorpusMetrics::default(),
            new_entry_indices: Default::default(),
            last_sync_timestamp: 0,
            worker_dir: Some(corpus_root),
            last_sync_metrics: CorpusMetrics::default(),
        };

        // Update top_rated and compute initial favored set
        let corpus_entries: Vec<_> = manager.in_memory_corpus.clone();
        for corpus in &corpus_entries {
            manager.update_top_rated(corpus);
        }

        let _ = manager.recompute_favored_and_cull_corpus();

        // First eviction should remove the non-favored one.
        assert_eq!(manager.in_memory_corpus.len(), 1);
        assert!(manager.in_memory_corpus[0].uuid == favored_uuid);
        assert!(manager.in_memory_corpus.iter().all(|c| c.is_favored));

        // Culling should be idempotent.
        manager.cull_corpus().unwrap();
        assert_eq!(manager.in_memory_corpus.len(), 1);
        assert!(manager.in_memory_corpus[0].uuid == favored_uuid);
        assert!(manager.in_memory_corpus.iter().all(|c| c.is_favored));
    }
}
