use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::{Bytes, U256};
use eyre::eyre;
use foundry_config::FuzzCorpusConfig;
use foundry_evm_fuzz::{
    BasicTxDetails,
    invariant::FuzzRunIdentifiedContracts,
    strategies::{EvmFuzzState, mutate_param_value},
};
use proptest::{
    prelude::{BoxedStrategy, Just, Rng, Strategy},
    prop_oneof,
    test_runner::TestRunner,
};
use uuid::Uuid;

use crate::executors::{
    Executor, RawCallResult,
    corpus::{CorpusEntry, CorpusMetrics, MutationType},
};

const METADATA_SUFFIX: &str = "metadata.json";
const JSON_EXTENSION: &str = ".json";
const FAVORABILITY_THRESHOLD: f64 = 0.3;
const COVERAGE_MAP_SIZE: usize = 65536;
const WORKER: &str = "worker";

/// Per-worker corpus manager.
pub struct WorkerCorpus {
    /// Worker Id
    id: u32,
    /// In-memory corpus entries populated from the persisted files and
    /// runs administered by this worker.
    in_memory_corpus: Vec<CorpusEntry>,
    /// History of binned hitcount of edges seen during fuzzing
    history_map: Vec<u8>,
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
}

impl WorkerCorpus {
    pub fn new(
        id: u32,
        master: &MasterCorpus,
        tx_generator: BoxedStrategy<BasicTxDetails>,
    ) -> eyre::Result<Self> {
        let config = master.config.clone();
        let mutation_generator = prop_oneof![
            Just(MutationType::Splice),
            Just(MutationType::Repeat),
            Just(MutationType::Interleave),
            Just(MutationType::Prefix),
            Just(MutationType::Suffix),
            Just(MutationType::Abi),
        ]
        .boxed();

        if let Some(corpus_dir) = &config.corpus_dir {
            let worker_dir = corpus_dir.join(format!("{WORKER}{id}"));

            if !worker_dir.is_dir() {
                foundry_common::fs::create_dir_all(worker_dir)?;
            }
        }

        Ok(Self {
            id,
            in_memory_corpus: master.in_memory_corpus.clone(),
            // TODO: This clones the history_map with size COVERAGE_MAP_SIZE
            // history_map size per worker should be dependent on the total number of workers?
            history_map: master.history_map.clone(),
            metrics: Default::default(),
            tx_generator,
            mutation_generator,
            current_mutated: None,
            config,
            new_entry_indices: Default::default(),
        })
    }

    /// Updates stats for the given call sequence, if new coverage produced.
    /// Persists the call sequence (if corpus directory is configured and new coverage) and updates
    /// in-memory corpus.
    pub fn process_inputs(&mut self, inputs: &[BasicTxDetails], new_coverage: bool) {
        // Early return if corpus dir / coverage guided fuzzing is not configured.
        let worker_dir = if let Some(corpus_dir) = &self.config.corpus_dir {
            corpus_dir.join(format!("{WORKER}{}", self.id))
        } else {
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
                    < FAVORABILITY_THRESHOLD;
                self.metrics.update_favored(is_favored, corpus.is_favored);
                corpus.is_favored = is_favored;

                trace!(
                    target: "corpus",
                    "updated worker {} corpus {}, total mutations: {}, new finds: {}",
                    self.id, corpus.uuid, corpus.total_mutations, corpus.new_finds_produced
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

        // Persist to disk if corpus dir is configured.
        let write_result = if self.config.corpus_gzip {
            foundry_common::fs::write_json_gzip_file(
                worker_dir.join(format!("{corpus_uuid}{JSON_EXTENSION}.gz")).as_path(),
                &corpus.tx_seq,
            )
        } else {
            foundry_common::fs::write_json_file(
                worker_dir.join(format!("{corpus_uuid}{JSON_EXTENSION}")).as_path(),
                &corpus.tx_seq,
            )
        };

        if let Err(err) = write_result {
            debug!(target: "corpus", %err, "Failed to record call sequence {:?} in worker {}", &corpus.tx_seq, self.id);
        } else {
            trace!(
                target: "corpus",
                "persisted {} inputs for new coverage in worker {} for {corpus_uuid} corpus",
                self.id, &corpus.tx_seq.len()
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
            let new_seq = corpus.tx_seq.clone();
            let mut tx = new_seq.first().unwrap().clone();
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
            debug!(target: "corpus", "evict corpus {uuid} in worker {}", self.id);

            // Flush to disk the seed metadata at the time of eviction.
            let eviction_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            foundry_common::fs::write_json_file(
                self.config
                    .corpus_dir
                    .clone()
                    .unwrap()
                    .join(format!("{WORKER}{}", self.id)) // Worker dir
                    .join(format!("{uuid}-{eviction_time}-{METADATA_SUFFIX}"))
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
}

/// Global corpus across workers to share coverage updates
pub struct MasterCorpus {
    /// Config
    config: Arc<FuzzCorpusConfig>,
    /// In-memory corpus entries populated from the persisted files. This is global corpus entry
    /// across workers.
    in_memory_corpus: Vec<CorpusEntry>,
    /// Number of failed replays from the persisted files.
    pub(crate) failed_replays: usize,
    /// History of binned hitcount of edges seen during fuzzing
    history_map: Vec<u8>,
    /// Master Metrics
    metrics: CorpusMetrics,
}

impl MasterCorpus {
    pub fn new(
        config: FuzzCorpusConfig,
        executor: &Executor,
        fuzzed_function: Option<&Function>,
        fuzzed_contracts: Option<&FuzzRunIdentifiedContracts>,
    ) -> eyre::Result<Self> {
        let mut history_map = vec![0u8; COVERAGE_MAP_SIZE];
        let mut metrics = CorpusMetrics::default();
        let mut in_memory_corpus = vec![];
        let mut failed_replays = 0;

        // Early return if corpus dir / coverage guided fuzzing not configured.
        let Some(corpus_dir) = &config.corpus_dir else {
            return Ok(Self {
                config: config.into(),
                in_memory_corpus,
                failed_replays,
                history_map,
                metrics,
            });
        };

        // Ensure corpus dir for current test is created.
        if !corpus_dir.is_dir() {
            foundry_common::fs::create_dir_all(corpus_dir)?;
        }

        let can_replay_tx = |tx: &BasicTxDetails| -> bool {
            fuzzed_contracts.is_some_and(|contracts| contracts.targets.lock().can_replay(tx))
                || fuzzed_function.is_some_and(|function| {
                    tx.call_details
                        .calldata
                        .get(..4)
                        .is_some_and(|selector| function.selector() == selector)
                })
        };

        'corpus_replay: for entry in std::fs::read_dir(corpus_dir)? {
            let path = entry?.path();
            if path.is_file()
                && let Some(name) = path.file_name().and_then(|s| s.to_str())
                && name.contains(METADATA_SUFFIX)
            {
                // Ignore metadata files
                continue;
            }

            let read_corpus_result = match path.extension().and_then(|ext| ext.to_str()) {
                Some("gz") => foundry_common::fs::read_json_gzip_file::<Vec<BasicTxDetails>>(&path),
                _ => foundry_common::fs::read_json_file::<Vec<BasicTxDetails>>(&path),
            };

            let Ok(tx_seq) = read_corpus_result else {
                trace!(target: "corpus", "failed to load corpus from {}", path.display());
                continue;
            };

            if !tx_seq.is_empty() {
                // Warm up history map from loaded sequences.
                let mut executor = executor.clone();
                for tx in &tx_seq {
                    if can_replay_tx(tx) {
                        let mut call_result = executor
                            .call_raw(
                                tx.sender,
                                tx.call_details.target,
                                tx.call_details.calldata.clone(),
                                U256::ZERO,
                            )
                            .map_err(|e| eyre!(format!("Could not make raw evm call: {e}")))?;

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
                    path.display()
                );

                // Populate in memory corpus with the sequence from corpus file.

                in_memory_corpus.push(CorpusEntry::new(tx_seq, path)?);
            }
        }

        Ok(Self { config: config.into(), in_memory_corpus, failed_replays, history_map, metrics })
    }
}
