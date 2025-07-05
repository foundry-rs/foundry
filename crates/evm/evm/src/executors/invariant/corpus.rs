use crate::executors::{
    Executor,
    invariant::{InvariantTest, InvariantTestRun},
};
use alloy_dyn_abi::JsonAbiExt;
use alloy_primitives::U256;
use eyre::eyre;
use foundry_config::InvariantConfig;
use foundry_evm_fuzz::{
    invariant::{BasicTxDetails, FuzzRunIdentifiedContracts},
    strategies::fuzz_param_from_state,
};
use proptest::{
    prelude::{Just, Rng, Strategy},
    prop_oneof,
    strategy::{BoxedStrategy, ValueTree},
    test_runner::TestRunner,
};
use serde::Serialize;
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const METADATA_SUFFIX: &str = "metadata.json";
const JSON_EXTENSION: &str = ".json";

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
#[derive(Serialize)]
struct Corpus {
    // Unique corpus identifier.
    uuid: Uuid,
    // Total mutations of corpus as primary source.
    total_mutations: usize,
    // New coverage found as a result of mutating this corpus.
    new_finds_produced: usize,
    // Corpus call sequence.
    #[serde(skip_serializing)]
    tx_seq: Vec<BasicTxDetails>,
}

impl Corpus {
    /// New corpus from given call sequence and corpus path to read uuid.
    pub fn new(tx_seq: Vec<BasicTxDetails>, path: PathBuf) -> eyre::Result<Self> {
        let uuid = if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            Uuid::try_from(stem.strip_suffix(JSON_EXTENSION).unwrap_or(stem).to_string())?
        } else {
            Uuid::new_v4()
        };
        Ok(Self { uuid, total_mutations: 0, new_finds_produced: 0, tx_seq })
    }

    /// New corpus with given call sequence and new uuid.
    pub fn from_tx_seq(tx_seq: Vec<BasicTxDetails>) -> Self {
        Self { uuid: Uuid::new_v4(), total_mutations: 0, new_finds_produced: 0, tx_seq }
    }
}

/// Invariant corpus manager.
pub struct TxCorpusManager {
    // Fuzzed calls generator.
    tx_generator: BoxedStrategy<BasicTxDetails>,
    // Call sequence mutation strategy type generator.
    mutation_generator: BoxedStrategy<MutationType>,
    // Path to invariant corpus directory. If None, sequences with new coverage are not persisted.
    corpus_dir: Option<PathBuf>,
    // Whether corpus to use gzip file compression and decompression.
    corpus_gzip: bool,
    // Number of mutations until entry marked as eligible to be flushed from in-memory corpus.
    // Mutations will be performed at least `corpus_min_mutations` times.
    corpus_min_mutations: usize,
    // Number of corpus that won't be evicted from memory.
    corpus_min_size: usize,
    // In-memory corpus, populated from persisted files and current runs.
    // Mutation is performed on these.
    in_memory_corpus: Vec<Corpus>,
    // Identifier of current mutated entry.
    current_mutated: Option<Uuid>,
    // Number of failed replays from persisted corpus.
    failed_replays: usize,
}

impl TxCorpusManager {
    pub fn new(
        invariant_config: &InvariantConfig,
        test_name: &String,
        fuzzed_contracts: &FuzzRunIdentifiedContracts,
        tx_generator: BoxedStrategy<BasicTxDetails>,
        executor: &Executor,
        history_map: &mut [u8],
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
        let mut in_memory_corpus = vec![];
        let corpus_gzip = invariant_config.corpus_gzip;
        let corpus_min_mutations = invariant_config.corpus_min_mutations;
        let corpus_min_size = invariant_config.corpus_min_size;
        let mut failed_replays = 0;

        // Early return if corpus dir / coverage guided fuzzing not configured.
        let Some(corpus_dir) = &invariant_config.corpus_dir else {
            return Ok(Self {
                tx_generator,
                mutation_generator,
                corpus_dir: None,
                corpus_gzip,
                corpus_min_mutations,
                corpus_min_size,
                in_memory_corpus,
                current_mutated: None,
                failed_replays,
            });
        };

        // Ensure corpus dir for invariant function is created.
        let corpus_dir = corpus_dir.join(test_name);
        if !corpus_dir.is_dir() {
            foundry_common::fs::create_dir_all(&corpus_dir)?;
        }

        let fuzzed_contracts = fuzzed_contracts.targets.lock();

        for entry in std::fs::read_dir(&corpus_dir)? {
            let path = entry?.path();
            if path.is_file()
                && let Some(name) = path.file_name().and_then(|s| s.to_str())
            {
                // Ignore metadata files
                if name.contains(METADATA_SUFFIX) {
                    continue;
                }
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
                    let mut call_result = executor
                        .call_raw(
                            tx.sender,
                            tx.call_details.target,
                            tx.call_details.calldata.clone(),
                            U256::ZERO,
                        )
                        .map_err(|e| eyre!(format!("Could not make raw evm call: {e}")))?;

                    if fuzzed_contracts.can_replay(tx) {
                        call_result.merge_edge_coverage(history_map);
                        executor.commit(&mut call_result);
                    } else {
                        failed_replays += 1;
                    }
                }

                trace!(
                    target: "corpus",
                    "load sequence with len {} from corpus file {}",
                    tx_seq.len(),
                    path.display()
                );

                // Populate in memory corpus with sequence from corpus file.
                in_memory_corpus.push(Corpus::new(tx_seq, path)?);
            }
        }

        Ok(Self {
            tx_generator,
            mutation_generator,
            corpus_dir: Some(corpus_dir),
            corpus_gzip,
            corpus_min_mutations,
            corpus_min_size,
            in_memory_corpus,
            current_mutated: None,
            failed_replays,
        })
    }

    /// Collects inputs from given invariant run, if new coverage produced.
    /// Persists call sequence (if corpus directory is configured) and updates in-memory corpus.
    pub fn collect_inputs(&mut self, test_run: &InvariantTestRun) {
        // Early return if corpus dir / coverage guided fuzzing is not configured.
        let Some(corpus_dir) = &self.corpus_dir else {
            return;
        };

        // Update stats of current mutated primary corpus.
        if let Some(uuid) = &self.current_mutated {
            if let Some(corpus) =
                self.in_memory_corpus.iter_mut().find(|corpus| corpus.uuid.eq(uuid))
            {
                corpus.total_mutations += 1;
                if test_run.new_coverage {
                    corpus.new_finds_produced += 1
                }

                trace!(
                    target: "corpus",
                    "updated corpus {}, total mutations: {}, new finds: {}",
                    corpus.uuid, corpus.total_mutations, corpus.new_finds_produced
                );
            }

            self.current_mutated = None;
        }

        // Collect inputs only if current run produced new coverage.
        if !test_run.new_coverage {
            return;
        }

        let corpus = Corpus::from_tx_seq(test_run.inputs.clone());
        let corpus_uuid = corpus.uuid;

        // Persist to disk if corpus dir is configured.
        let write_result = if self.corpus_gzip {
            foundry_common::fs::write_json_gzip_file(
                corpus_dir.join(format!("{corpus_uuid}{JSON_EXTENSION}.gz")).as_path(),
                &corpus.tx_seq,
            )
        } else {
            foundry_common::fs::write_json_file(
                corpus_dir.join(format!("{corpus_uuid}{JSON_EXTENSION}")).as_path(),
                &corpus.tx_seq,
            )
        };

        if let Err(err) = write_result {
            debug!(target: "corpus", %err, "Failed to record call sequence {:?}", &corpus.tx_seq);
        } else {
            trace!(
                target: "corpus",
                "persisted {} inputs for new coverage in {corpus_uuid} corpus",
                &corpus.tx_seq.len()
            );
        }

        // This includes reverting txs in the corpus and `can_continue` removes
        // them. We want this as it is new coverage and may help reach the other branch.
        self.in_memory_corpus.push(corpus);
    }

    /// Generates new call sequence from in memory corpus. Evicts oldest corpus mutated more than
    /// configured max mutations value.
    pub fn new_sequence(&mut self, test: &InvariantTest) -> eyre::Result<Vec<BasicTxDetails>> {
        let mut new_seq = vec![];
        let test_runner = &mut test.execution_data.borrow_mut().branch_runner;

        // Early return with first_input only if corpus dir / coverage guided fuzzing not
        // configured.
        let Some(corpus_dir) = &self.corpus_dir else {
            new_seq.push(self.new_tx(test_runner)?);
            return Ok(new_seq);
        };

        if !self.in_memory_corpus.is_empty() {
            // Flush oldest corpus mutated more than configured max mutations unless they are
            // producing new finds more than 1/3 of the time.
            let should_evict = self.in_memory_corpus.len() > self.corpus_min_size.max(1);
            if should_evict
                && let Some(index) = self.in_memory_corpus.iter().position(|corpus| {
                    corpus.total_mutations > self.corpus_min_mutations
                        && (corpus.new_finds_produced as f64 / corpus.total_mutations as f64) < 0.3
                })
            {
                let corpus = self.in_memory_corpus.get(index).unwrap();
                let uuid = corpus.uuid;
                debug!(target: "corpus", "evict corpus {uuid}");

                // Flush to disk the seed metadata at the time of eviction.
                let eviction_time = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs();
                foundry_common::fs::write_json_file(
                    corpus_dir.join(format!("{uuid}-{eviction_time}-{METADATA_SUFFIX}")).as_path(),
                    &corpus,
                )?;
                // Remove corpus from memory.
                self.in_memory_corpus.remove(index);
            }

            let mutation_type = self
                .mutation_generator
                .new_tree(test_runner)
                .expect("Could not generate mutation type")
                .current();
            let rng = test_runner.rng();
            let corpus_len = self.in_memory_corpus.len();
            let primary = &self.in_memory_corpus[rng.random_range(0..corpus_len)];
            let secondary = &self.in_memory_corpus[rng.random_range(0..corpus_len)];

            match mutation_type {
                MutationType::Splice => {
                    trace!(target: "corpus", "splice {} and {}", primary.uuid, secondary.uuid);
                    if should_evict {
                        self.current_mutated = Some(primary.uuid);
                    }
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
                    if should_evict {
                        self.current_mutated = Some(corpus.uuid);
                    }
                    new_seq = corpus.tx_seq.clone();
                    let start = rng.random_range(0..corpus.tx_seq.len());
                    let end = rng.random_range(start..corpus.tx_seq.len());
                    let item_idx = rng.random_range(0..corpus.tx_seq.len());
                    let repeated = vec![new_seq[item_idx].clone(); end - start];
                    new_seq.splice(start..end, repeated);
                }
                MutationType::Interleave => {
                    trace!(target: "corpus", "interleave {} with {}", primary.uuid, secondary.uuid);
                    if should_evict {
                        self.current_mutated = Some(primary.uuid);
                    }
                    for (tx1, tx2) in primary.tx_seq.iter().zip(secondary.tx_seq.iter()) {
                        // chunks?
                        let tx = if rng.random::<bool>() { tx1.clone() } else { tx2.clone() };
                        new_seq.push(tx);
                    }
                }
                MutationType::Prefix => {
                    let corpus = if rng.random::<bool>() { primary } else { secondary };
                    trace!(target: "corpus", "overwrite prefix of {}", corpus.uuid);
                    if should_evict {
                        self.current_mutated = Some(corpus.uuid);
                    }
                    new_seq = corpus.tx_seq.clone();
                    for i in 0..rng.random_range(0..=new_seq.len()) {
                        new_seq[i] = self.new_tx(test_runner)?;
                    }
                }
                MutationType::Suffix => {
                    let corpus = if rng.random::<bool>() { primary } else { secondary };
                    trace!(target: "corpus", "overwrite suffix of {}", corpus.uuid);
                    if should_evict {
                        self.current_mutated = Some(corpus.uuid);
                    }
                    new_seq = corpus.tx_seq.clone();
                    for i in new_seq.len() - rng.random_range(0..new_seq.len())..corpus.tx_seq.len()
                    {
                        new_seq[i] = self.new_tx(test_runner)?;
                    }
                }
                MutationType::Abi => {
                    let targets = test.targeted_contracts.targets.lock();
                    let corpus = if rng.random::<bool>() { primary } else { secondary };
                    trace!(target: "corpus", "ABI mutate args of {}", corpus.uuid);
                    if should_evict {
                        self.current_mutated = Some(corpus.uuid);
                    }
                    new_seq = corpus.tx_seq.clone();

                    let idx = rng.random_range(0..new_seq.len());
                    let tx = new_seq.get_mut(idx).unwrap();
                    if let (_, Some(function)) = targets.fuzzed_artifacts(tx) {
                        // TODO add call_value to call details and mutate it as well as sender some
                        // of the time
                        if !function.inputs.is_empty() {
                            let mut new_function = function.clone();
                            let mut arg_mutation_rounds =
                                rng.random_range(0..=function.inputs.len()).max(1);
                            let round_arg_idx: Vec<usize> = if function.inputs.len() <= 1 {
                                vec![0]
                            } else {
                                (0..arg_mutation_rounds)
                                    .map(|_| {
                                        test_runner.rng().random_range(0..function.inputs.len())
                                    })
                                    .collect()
                            };
                            // TODO mutation strategy for individual ABI types
                            let mut prev_inputs = function
                                .abi_decode_input(&tx.call_details.calldata[4..])
                                .expect("fuzzed_artifacts returned wrong sig");
                            // For now, only new inputs are generated, no existing inputs are
                            // mutated.
                            let mut gen_input = |input: &alloy_json_abi::Param| {
                                fuzz_param_from_state(
                                    &input.selector_type().parse().unwrap(),
                                    &test.fuzz_state,
                                )
                                .new_tree(test_runner)
                                .expect("Could not generate case")
                                .current()
                            };

                            while arg_mutation_rounds > 0 {
                                let idx = round_arg_idx[arg_mutation_rounds - 1];
                                let input = new_function
                                    .inputs
                                    .get_mut(idx)
                                    .expect("Could not get input to mutate");
                                let new_input = gen_input(input);
                                prev_inputs[idx] = new_input;
                                arg_mutation_rounds -= 1;
                            }

                            tx.call_details.calldata = new_function
                                .abi_encode_input(&prev_inputs)
                                .map_err(|e| eyre!(e.to_string()))?
                                .into();
                        }
                    }
                }
            }
        }

        // Make sure sequence contains at least one tx to start fuzzing from.
        if new_seq.is_empty() {
            new_seq.push(self.new_tx(test_runner)?);
        }
        trace!(target: "corpus", "new sequence of {} calls generated", new_seq.len());

        Ok(new_seq)
    }

    /// Returns the next call to be used in call sequence.
    /// If coverage guided fuzzing is not configured or if previous input was discarded then this is
    /// a new tx from strategy.
    /// If running with coverage guided fuzzing it returns a new call only when sequence
    /// does not have enough entries, or randomly. Otherwise, returns the next call from initial
    /// sequence.
    pub fn generate_next_input(
        &mut self,
        test: &InvariantTest,
        sequence: &[BasicTxDetails],
        discarded: bool,
        depth: usize,
    ) -> eyre::Result<BasicTxDetails> {
        let test_runner = &mut test.execution_data.borrow_mut().branch_runner;

        // Early return with new input if corpus dir / coverage guided fuzzing not configured or if
        // call was discarded.
        if self.corpus_dir.is_none() || discarded {
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

    /// Generates single call from invariant strategy.
    pub fn new_tx(&mut self, test_runner: &mut TestRunner) -> eyre::Result<BasicTxDetails> {
        Ok(self
            .tx_generator
            .new_tree(test_runner)
            .map_err(|_| eyre!("Could not generate case"))?
            .current())
    }

    pub fn failed_replays(self) -> usize {
        self.failed_replays
    }
}
