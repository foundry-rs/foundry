use crate::executors::{Executor, RawCallResult, invariant::execute_tx};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::Bytes;
use eyre::eyre;
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

const METADATA_SUFFIX: &str = "metadata.json";
const JSON_EXTENSION: &str = ".json";
const FAVORABILITY_THRESHOLD: f64 = 0.3;
const COVERAGE_MAP_SIZE: usize = 65536;
const WORKER: &str = "worker";
const CORPUS_DIR: &str = "corpus";
const SYNC_DIR: &str = "sync";

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
    // Total mutations of corpus as primary source.
    total_mutations: usize,
    // New coverage found as a result of mutating this corpus.
    new_finds_produced: usize,
    // Corpus call sequence.
    #[serde(skip_serializing)]
    tx_seq: Vec<BasicTxDetails>,
    // Whether this corpus is favored, i.e. producing new finds more often than
    // `FAVORABILITY_THRESHOLD`.
    is_favored: bool,
    /// Timestamp of when this entry was written to disk in seconds.
    #[serde(skip_serializing)]
    timestamp: u64,
}

impl CorpusEntry {
    /// New corpus from given call sequence and corpus path to read uuid.
    pub fn new(tx_seq: Vec<BasicTxDetails>, path: PathBuf) -> eyre::Result<Self> {
        Self::_new(tx_seq, Some(path))
    }

    /// New corpus with given call sequence and new uuid.
    pub fn from_tx_seq(tx_seq: &[BasicTxDetails]) -> Self {
        Self::_new(tx_seq.into(), None).expect("infallible")
    }

    fn _new(tx_seq: Vec<BasicTxDetails>, path: Option<PathBuf>) -> eyre::Result<Self> {
        let uuid = if let Some(path) = path
            && let Some(stem) = path.file_stem()
            && let Some(stem) = stem.to_str()
        {
            Uuid::try_from(stem.strip_suffix(JSON_EXTENSION).unwrap_or(stem).to_string())?
        } else {
            Uuid::new_v4()
        };
        Ok(Self {
            uuid,
            total_mutations: 0,
            new_finds_produced: 0,
            tx_seq,
            is_favored: false,
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        })
    }
}

#[derive(Serialize, Default)]
pub(crate) struct GlobalCorpusMetrics {
    // Number of edges seen during the invariant run
    cumulative_edges_seen: AtomicUsize,
    // Number of features (new hitcount bin of previously hit edge) seen during the invariant run
    cumulative_features_seen: AtomicUsize,
    // Number of corpus entries
    corpus_count: AtomicUsize,
    // Number of corpus entries that are favored
    favored_items: AtomicUsize,
}

impl fmt::Display for GlobalCorpusMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.load().fmt(f)
    }
}

impl GlobalCorpusMetrics {
    fn load(&self) -> CorpusMetrics {
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

    /// Updates campaign favored items.
    pub fn update_favored(&mut self, is_favored: bool, corpus_favored: bool) {
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
    id: u32,
    /// In-memory corpus entries populated from the persisted files and
    /// runs administered by this worker.
    in_memory_corpus: Vec<CorpusEntry>,
    /// History of binned hitcount of edges seen during fuzzing
    history_map: Vec<u8>,
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
        id: u32,
        config: FuzzCorpusConfig,
        tx_generator: BoxedStrategy<BasicTxDetails>,
        // Only required by master worker (id = 0) to replay existing corpus
        executor: Option<&Executor>,
        fuzzed_function: Option<&Function>,
        fuzzed_contracts: Option<&FuzzRunIdentifiedContracts>,
    ) -> eyre::Result<Self> {
        let mutation_generator = prop_oneof![
            Just(MutationType::Splice),
            Just(MutationType::Repeat),
            Just(MutationType::Interleave),
            Just(MutationType::Prefix),
            Just(MutationType::Suffix),
            Just(MutationType::Abi),
        ]
        .boxed();

        let worker_dir = if let Some(corpus_dir) = &config.corpus_dir {
            // Create the necessary directories for the worker
            let worker_dir = corpus_dir.join(format!("{WORKER}{id}"));
            let worker_corpus = &worker_dir.join(CORPUS_DIR);
            let sync_dir = &worker_dir.join(SYNC_DIR);

            if !worker_corpus.is_dir() {
                foundry_common::fs::create_dir_all(worker_corpus)?;
            }

            if !sync_dir.is_dir() {
                foundry_common::fs::create_dir_all(sync_dir)?;
            }

            Some(worker_dir)
        } else {
            None
        };

        let mut in_memory_corpus = vec![];
        let mut history_map = vec![0u8; COVERAGE_MAP_SIZE];
        let mut metrics = CorpusMetrics::default();
        let mut failed_replays = 0;

        if id == 0
            && let Some(corpus_dir) = &config.corpus_dir
        {
            // Master worker loads the initial corpus, if it exists.
            // Then, [distribute]s it to workers.
            let can_replay_tx = |tx: &BasicTxDetails| -> bool {
                fuzzed_contracts.is_some_and(|contracts| contracts.targets.lock().can_replay(tx))
                    || fuzzed_function.is_some_and(|function| {
                        tx.call_details
                            .calldata
                            .get(..4)
                            .is_some_and(|selector| function.selector() == selector)
                    })
            };

            let executor = executor.expect("Executor required for master worker");
            'corpus_replay: for entry in read_corpus_dir(corpus_dir) {
                if entry.is_metadata() {
                    continue;
                }
                let tx_seq = entry.read_tx_seq()?;
                if tx_seq.is_empty() {
                    continue;
                }
                // Warm up history map from loaded sequences.
                let mut executor = executor.clone();
                for tx in &tx_seq {
                    if can_replay_tx(tx) {
                        let mut call_result = execute_tx(&mut executor, tx)?;
                        let (new_coverage, is_edge) =
                            call_result.merge_edge_coverage(&mut history_map);
                        if new_coverage {
                            metrics.update_seen(is_edge);
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

                metrics.corpus_count += 1;

                trace!(
                    target: "corpus",
                    "load sequence with len {} from corpus file {}",
                    tx_seq.len(),
                    entry.path.display()
                );

                // Populate in memory corpus with the sequence from corpus file.
                in_memory_corpus.push(CorpusEntry::new(tx_seq, entry.path)?);
            }
        }

        Ok(Self {
            id,
            in_memory_corpus,
            history_map,
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
        })
    }

    /// Updates stats for the given call sequence, if new coverage produced.
    /// Persists the call sequence (if corpus directory is configured and new coverage) and updates
    /// in-memory corpus.
    #[tracing::instrument(skip_all, fields(worker_id = self.id))]
    pub fn process_inputs(&mut self, inputs: &[BasicTxDetails], new_coverage: bool) {
        let Some(worker_corpus) = &self.worker_dir else {
            return;
        };

        // Update stats of current mutated primary corpus.
        if let Some(uuid) = &self.current_mutated {
            if let Some(corpus) =
                self.in_memory_corpus.iter_mut().find(|corpus| corpus.uuid.eq(uuid))
            {
                corpus.total_mutations += 1;
                if new_coverage {
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

            self.current_mutated = None;
        }

        // Collect inputs only if current run produced new coverage.
        if !new_coverage {
            return;
        }

        let corpus = CorpusEntry::from_tx_seq(inputs);
        let corpus_uuid = corpus.uuid;
        let timestamp = corpus.timestamp;
        // Persist to disk if corpus dir is configured.
        let write_result = if self.config.corpus_gzip {
            foundry_common::fs::write_json_gzip_file(
                worker_corpus
                    .join(format!("{corpus_uuid}-{timestamp}{JSON_EXTENSION}.gz"))
                    .as_path(),
                &corpus.tx_seq,
            )
        } else {
            foundry_common::fs::write_json_file(
                worker_corpus.join(format!("{corpus_uuid}-{timestamp}{JSON_EXTENSION}")).as_path(),
                &corpus.tx_seq,
            )
        };

        if let Err(err) = write_result {
            debug!(target: "corpus", %err, "Failed to record call sequence {:?}", &corpus.tx_seq);
        } else {
            trace!(
                target: "corpus",
                "persisted {} inputs for new coverage for {corpus_uuid} corpus",
                &corpus.tx_seq.len()
            );
        }

        // Track in-memory corpus changes to update MasterWorker on sync
        let new_index = self.in_memory_corpus.len();
        self.new_entry_indices.push(new_index);

        // This includes reverting txs in the corpus and `can_continue` removes
        // them. We want this as it is new coverage and may help reach the other branch.
        self.metrics.corpus_count += 1;
        self.in_memory_corpus.push(corpus);
    }

    /// Collects coverage from call result and updates metrics.
    pub fn merge_edge_coverage(&mut self, call_result: &mut RawCallResult) -> bool {
        if !self.config.collect_edge_coverage() {
            return false;
        }

        let (new_coverage, is_edge) = call_result.merge_edge_coverage(&mut self.history_map);
        if new_coverage {
            self.metrics.update_seen(is_edge);
        }
        new_coverage
    }

    /// Generates new call sequence from in memory corpus. Evicts oldest corpus mutated more than
    /// configured max mutations value. Used by invariant test campaigns.
    #[tracing::instrument(skip_all, fields(worker_id = self.id))]
    pub fn new_inputs(
        &mut self,
        test_runner: &mut TestRunner,
        fuzz_state: &EvmFuzzState,
        targeted_contracts: &FuzzRunIdentifiedContracts,
    ) -> eyre::Result<Vec<BasicTxDetails>> {
        let mut new_seq = vec![];

        // Early return with first_input only if corpus dir / coverage guided fuzzing not
        // configured.
        if !self.config.is_coverage_guided() {
            new_seq.push(self.new_tx(test_runner)?);
            return Ok(new_seq);
        };

        if !self.in_memory_corpus.is_empty() {
            self.evict_oldest_corpus()?;

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
                        // chunks?
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
                        // TODO add call_value to call details and mutate it as well as sender some
                        // of the time
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

    /// Generates a new input from the shared in memory corpus.  Evicts oldest corpus mutated more
    /// than configured max mutations value. Used by fuzz (stateless) test campaigns.
    #[tracing::instrument(skip_all, fields(worker_id = self.id))]
    pub fn new_input(
        &mut self,
        test_runner: &mut TestRunner,
        fuzz_state: &EvmFuzzState,
        function: &Function,
    ) -> eyre::Result<Bytes> {
        // Early return if not running with coverage guided fuzzing.
        if !self.config.is_coverage_guided() {
            return Ok(self.new_tx(test_runner)?.call_details.calldata);
        }

        self.evict_oldest_corpus()?;

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
    pub fn new_tx(&self, test_runner: &mut TestRunner) -> eyre::Result<BasicTxDetails> {
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
    ) -> eyre::Result<BasicTxDetails> {
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

        // Continue with the next call initial sequence
        Ok(sequence[depth].clone())
    }

    /// Flush the oldest corpus mutated more than configured max mutations unless they are
    /// favored.
    fn evict_oldest_corpus(&mut self) -> eyre::Result<()> {
        if self.in_memory_corpus.len() > self.config.corpus_min_size.max(1)
            && let Some(index) = self.in_memory_corpus.iter().position(|corpus| {
                corpus.total_mutations > self.config.corpus_min_mutations && !corpus.is_favored
            })
        {
            let corpus = self.in_memory_corpus.get(index).unwrap();

            let uuid = corpus.uuid;
            debug!(target: "corpus", "evict corpus {uuid}");

            // Flush to disk the seed metadata at the time of eviction.
            let eviction_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            foundry_common::fs::write_json_file(
                self.worker_dir
                    .clone()
                    .unwrap()
                    .join(format!("{uuid}-{eviction_time}.{METADATA_SUFFIX}"))
                    .as_path(),
                &corpus,
            )?;

            // Remove corpus from memory.
            self.in_memory_corpus.remove(index);

            // Adjust the tracked indices
            self.new_entry_indices.retain_mut(|i| {
                if *i > index {
                    *i -= 1; // Shift indices down
                    true // Keep this index
                } else {
                    *i != index // Remove if it's the deleted index, keep otherwise
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
        fuzz_state: &EvmFuzzState,
    ) -> eyre::Result<()> {
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

    // Sync Methods

    /// Exports the new corpus entries to the master worker's (id = 0) sync dir.
    #[tracing::instrument(skip_all, fields(worker_id = self.id))]
    fn export(&self) -> eyre::Result<()> {
        // Early return if no new entries or corpus dir not configured
        if self.new_entry_indices.is_empty() || self.worker_dir.is_none() {
            return Ok(());
        }

        let worker_dir = self.worker_dir.as_ref().unwrap();

        // Master doesn't export (it only receives from others)
        if self.id == 0 {
            return Ok(());
        }

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
            if let Some(entry) = self.in_memory_corpus.get(index) {
                // TODO(dani): with_added_extension
                let ext = if self.config.corpus_gzip {
                    &format!("{JSON_EXTENSION}.gz")
                } else {
                    JSON_EXTENSION
                };
                let file_name = format!("{}-{}{ext}", entry.uuid, entry.timestamp);
                let file_path = corpus_dir.join(&file_name);
                let sync_path = master_sync_dir.join(&file_name);

                // TODO(dani): can we symlink and copy later instead?
                let Ok(_) = foundry_common::fs::copy(file_path, sync_path) else {
                    debug!(target: "corpus", "failed to export corpus {}", entry.uuid);
                    continue;
                };

                exported += 1;
            }
        }

        trace!(target: "corpus", "exported {exported} new corpus entries");

        Ok(())
    }

    /// Imports the new corpus entries tx sequence which will be used to replay and update history
    /// map.
    fn import(&self) -> eyre::Result<Vec<Vec<BasicTxDetails>>> {
        let Some(worker_dir) = &self.worker_dir else {
            return Ok(vec![]);
        };

        let sync_dir = worker_dir.join(SYNC_DIR);
        if !sync_dir.is_dir() {
            return Ok(vec![]);
        }

        let mut imports = vec![];
        for entry in read_corpus_dir(&sync_dir) {
            debug_assert!(
                !entry.is_metadata(),
                "there should be no metadata in sync dir {sync_dir:?}",
            );
            if entry.timestamp <= self.last_sync_timestamp {
                continue;
            }
            imports.push(entry.read_tx_seq()?);
        }

        Ok(imports)
    }

    /// Syncs and calibrates the in memory corpus and updates the history_map if new coverage is
    /// found from the corpus findings of other workers.
    #[tracing::instrument(skip_all, fields(worker_id = self.id))]
    fn calibrate(
        &mut self,
        executor: &Executor,
        fuzzed_function: Option<&Function>,
        fuzzed_contracts: Option<&FuzzRunIdentifiedContracts>,
    ) -> eyre::Result<()> {
        let Some(worker_dir) = &self.worker_dir else {
            return Ok(());
        };

        // Helper to check if tx can be replayed
        let can_replay_tx = |tx: &BasicTxDetails| -> bool {
            fuzzed_contracts.is_some_and(|contracts| contracts.targets.lock().can_replay(tx))
                || fuzzed_function.is_some_and(|function| {
                    tx.call_details
                        .calldata
                        .get(..4)
                        .is_some_and(|selector| function.selector() == selector)
                })
        };

        let sync_dir = worker_dir.join(SYNC_DIR);
        let corpus_dir = worker_dir.join(CORPUS_DIR);

        let mut executor = executor.clone();
        for tx_seq in self.import()? {
            if !tx_seq.is_empty() {
                let mut new_coverage_on_sync = false;
                for tx in &tx_seq {
                    if can_replay_tx(tx) {
                        let mut call_result = execute_tx(&mut executor, tx)?;

                        // Check if this provides new coverage
                        let (new_coverage, is_edge) =
                            call_result.merge_edge_coverage(&mut self.history_map);

                        if new_coverage {
                            self.metrics.update_seen(is_edge);
                            new_coverage_on_sync = true;
                        }

                        // Commit only for stateful tests
                        if fuzzed_contracts.is_some() {
                            executor.commit(&mut call_result);
                        }

                        trace!(
                            target: "corpus",
                            %new_coverage,
                            "replayed tx for syncing: {:?}",
                            &tx
                        );
                    }
                }

                if new_coverage_on_sync {
                    let corpus_entry = CorpusEntry::from_tx_seq(&tx_seq);
                    let ext = self
                        .config
                        .corpus_gzip
                        .then_some(format!("{JSON_EXTENSION}.gz"))
                        .unwrap_or(JSON_EXTENSION.to_string());

                    let file_name =
                        format!("{}-{}{ext}", corpus_entry.uuid, corpus_entry.timestamp);

                    // Move file from sync/ to corpus/ directory
                    let sync_path = sync_dir.join(&file_name);
                    let corpus_path = corpus_dir.join(&file_name);

                    let Ok(_) = std::fs::rename(&sync_path, &corpus_path) else {
                        debug!(target: "corpus", "failed to move synced corpus {} from {sync_path:?} to {corpus_path:?} dir", corpus_entry.uuid);
                        continue;
                    };

                    trace!(
                        target: "corpus",
                        "moved synced corpus {} to corpus dir",
                        corpus_entry.uuid
                    );

                    self.in_memory_corpus.push(corpus_entry);
                }
            }
        }

        let last_sync = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        trace!(target: "corpus", "sync complete, updating last sync time to {}", last_sync);
        self.last_sync_timestamp = last_sync;

        Ok(())
    }

    /// To be run by the master worker (id = 0) to distribute the global corpus to sync/ directories
    /// of other workers.
    #[tracing::instrument(skip_all, fields(worker_id = self.id))]
    fn distribute(&mut self, num_workers: u32) -> eyre::Result<()> {
        if self.id != 0 || self.worker_dir.is_none() {
            return Ok(());
        }

        let worker_dir = self.worker_dir.as_ref().unwrap();
        let master_corpus_dir = worker_dir.join(CORPUS_DIR);
        let master_corpus = read_corpus_dir(&master_corpus_dir).collect::<Vec<_>>();

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

            for entry in &master_corpus {
                if entry.is_metadata() {
                    continue;
                }
                let name = entry.name();
                let sync_path = target_dir.join(name);
                if entry.timestamp <= self.last_sync_timestamp {
                    continue;
                }

                match foundry_common::fs::copy(&entry.path, &sync_path) {
                    Ok(_) => {
                        trace!(target: "corpus", %name, ?target_dir, "distributed corpus");
                    }
                    Err(err) => {
                        trace!(target: "corpus", %name, %err, "failed to distribute corpus");
                        continue;
                    }
                }
            }
        }

        Ok(())
    }

    /// Syncs local metrics with global corpus metrics by calculating and applying deltas
    pub(crate) fn sync_metrics(&mut self, global_corpus_metrics: &GlobalCorpusMetrics) {
        // Calculate delta metrics since last sync
        let edges_delta = self
            .metrics
            .cumulative_edges_seen
            .saturating_sub(self.last_sync_metrics.cumulative_edges_seen);
        let features_delta = self
            .metrics
            .cumulative_features_seen
            .saturating_sub(self.last_sync_metrics.cumulative_features_seen);
        // For corpus count and favored items, calculate deltas
        let corpus_count_delta =
            self.metrics.corpus_count as isize - self.last_sync_metrics.corpus_count as isize;
        let favored_delta =
            self.metrics.favored_items as isize - self.last_sync_metrics.favored_items as isize;

        // Add delta values to global metrics

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

        // Store current metrics as last sync metrics for next delta calculation
        self.last_sync_metrics = self.metrics.clone();
    }

    /// Syncs the workers in_memory_corpus and history_map with the findings from other workers.
    #[tracing::instrument(skip_all, fields(worker_id = self.id))]
    pub fn sync(
        &mut self,
        num_workers: u32,
        executor: &Executor,
        fuzzed_function: Option<&Function>,
        fuzzed_contracts: Option<&FuzzRunIdentifiedContracts>,
        global_corpus_metrics: &GlobalCorpusMetrics,
    ) -> eyre::Result<()> {
        self.sync_metrics(global_corpus_metrics);

        let is_master = self.id == 0;
        if !is_master {
            self.export()?;
        }
        self.calibrate(executor, fuzzed_function, fuzzed_contracts)?;
        if is_master {
            self.distribute(num_workers)?;
        }
        self.new_entry_indices.clear();
        trace!(target: "corpus", "synced");
        Ok(())
    }
}

fn read_corpus_dir(path: &Path) -> impl Iterator<Item = CorpusDirEntry> {
    let dir = match std::fs::read_dir(path) {
        Ok(dir) => dir,
        Err(err) => {
            trace!(%err, ?path, "failed to read corpus directory");
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
                trace!(target: "corpus", ?path, "failed to parse corpus filename");
                None
            }
        }
        Err(err) => {
            trace!(%err, "failed to read corpus directory entry");
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

    fn is_metadata(&self) -> bool {
        self.name().contains(METADATA_SUFFIX)
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
fn parse_corpus_filename(name: &str) -> eyre::Result<(Uuid, u64)> {
    let name = name.trim_end_matches(".gz").trim_end_matches(".json").trim_end_matches(".metadata");

    let parts = name.rsplitn(2, "-").collect::<Vec<_>>();
    let uuid = Uuid::parse_str(parts[0])?;
    let timestamp = parts[1].parse()?;

    Ok((uuid, timestamp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Address;
    use std::fs;

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
        let corpus = CorpusEntry::from_tx_seq(&tx_seq);
        let seed_uuid = corpus.uuid;

        // Create corpus root dir and worker subdirectory
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
            metrics: CorpusMetrics::default(),
            new_entry_indices: Default::default(),
            last_sync_timestamp: 0,
            worker_dir: Some(corpus_root),
            last_sync_metrics: CorpusMetrics::default(),
        };

        (manager, seed_uuid)
    }

    #[test]
    fn favored_sets_true_and_metrics_increment_when_ratio_gt_threshold() {
        let (mut manager, uuid) = new_manager_with_single_corpus();
        let corpus = manager.in_memory_corpus.iter_mut().find(|c| c.uuid == uuid).unwrap();
        corpus.total_mutations = 4;
        corpus.new_finds_produced = 2; // ratio currently 0.5 if both increment → 3/5 = 0.6 > 0.3
        corpus.is_favored = false;

        // ensure metrics start at 0
        assert_eq!(manager.metrics.favored_items, 0);

        // mark this as the currently mutated corpus and process a run with new coverage
        manager.current_mutated = Some(uuid);
        manager.process_inputs(&[basic_tx()], true);

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
        corpus.new_finds_produced = 3; // 3/9 = 0.333.. > 0.3; after +1: 3/10 = 0.3 => not favored
        corpus.is_favored = true; // start as favored

        manager.metrics.favored_items = 1;

        // Next run does NOT produce coverage → only total_mutations increments, ratio drops
        manager.current_mutated = Some(uuid);
        manager.process_inputs(&[basic_tx()], false);

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
        // After this call with new_coverage=true, totals become 10 and 3 → 0.3
        corpus.total_mutations = 9;
        corpus.new_finds_produced = 2;
        corpus.is_favored = false;

        manager.current_mutated = Some(uuid);
        manager.process_inputs(&[basic_tx()], true);

        let corpus = manager.in_memory_corpus.iter().find(|c| c.uuid == uuid).unwrap();
        assert!(
            !(corpus.is_favored),
            "with strict '>' comparison, favored must be false when ratio == threshold"
        );
    }

    #[test]
    fn eviction_skips_favored_and_evicts_non_favored() {
        // manager with two corpora
        let tx_gen = Just(basic_tx()).boxed();
        let config = FuzzCorpusConfig {
            corpus_dir: Some(temp_corpus_dir()),
            corpus_min_mutations: 0,
            corpus_min_size: 0,
            ..Default::default()
        };

        let mut favored = CorpusEntry::from_tx_seq(&[basic_tx()]);
        favored.total_mutations = 2;
        favored.is_favored = true;

        let mut non_favored = CorpusEntry::from_tx_seq(&[basic_tx()]);
        non_favored.total_mutations = 2;
        non_favored.is_favored = false;
        let non_favored_uuid = non_favored.uuid;

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
            metrics: CorpusMetrics::default(),
            new_entry_indices: Default::default(),
            last_sync_timestamp: 0,
            worker_dir: Some(corpus_root),
            last_sync_metrics: CorpusMetrics::default(),
        };

        // First eviction should remove the non-favored one
        manager.evict_oldest_corpus().unwrap();
        assert_eq!(manager.in_memory_corpus.len(), 1);
        assert!(manager.in_memory_corpus.iter().all(|c| c.is_favored));

        // Attempt eviction again: only favored remains → should not remove
        manager.evict_oldest_corpus().unwrap();
        assert_eq!(manager.in_memory_corpus.len(), 1, "favored corpus must not be evicted");

        // ensure the evicted one was the non-favored uuid
        assert!(manager.in_memory_corpus.iter().all(|c| c.uuid != non_favored_uuid));
    }
}
