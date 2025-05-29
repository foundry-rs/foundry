use crate::executors::{
    invariant::{InvariantTest, InvariantTestRun},
    Executor,
};
use alloy_primitives::U256;
use eyre::eyre;
use foundry_config::InvariantConfig;
use foundry_evm_fuzz::{
    invariant::{BasicTxDetails, FuzzRunIdentifiedContracts},
    strategies::fuzz_calldata,
    FuzzFixtures,
};
use proptest::{
    prelude::{Rng, Strategy},
    strategy::{BoxedStrategy, ValueTree},
    test_runner::TestRunner,
};
use serde::Serialize;
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

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
            Uuid::try_from(stem.to_string())?
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
    // Path to invariant corpus directory. If None, corpus with new coverage is not persisted.
    corpus_dir: Option<PathBuf>,
    // Whether corpus to use gzip file compression and decompression.
    corpus_gzip: bool,
    // Number of corpus mutations until marked as eligible to be flushed from in-memory corpus.
    corpus_max_mutations: usize,
    // Number of corpus that won't be evicted from memory.
    corpus_min_size: usize,
    // In-memory corpus, populated from persisted files and current runs.
    // Oldest corpus that is mutated more than `corpus_max_mutations` times.
    in_memory_corpus: Vec<Corpus>,
    // Identifier of current mutated corpus.
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
        let mut in_memory_corpus = vec![];
        let corpus_gzip = invariant_config.corpus_gzip;
        let corpus_max_mutations = invariant_config.corpus_max_mutations;
        let corpus_min_size = invariant_config.corpus_min_size;
        let mut failed_replays = 0;

        // Early return if corpus dir not configured.
        let Some(corpus_dir) = &invariant_config.corpus_dir else {
            return Ok(Self {
                tx_generator,
                corpus_dir: None,
                corpus_gzip,
                corpus_max_mutations,
                corpus_min_size,
                in_memory_corpus,
                current_mutated: None,
                failed_replays,
            })
        };

        // Ensure corpus dir for invariant function is created.
        let corpus_dir = corpus_dir.join(test_name);
        if !corpus_dir.is_dir() {
            foundry_common::fs::create_dir_all(&corpus_dir)?;
        }

        let fuzzed_contracts = fuzzed_contracts.targets.lock();

        for entry in std::fs::read_dir(&corpus_dir)? {
            let path = entry?.path();
            let read_corpus_result = if corpus_gzip {
                foundry_common::fs::read_json_gzip_file::<Vec<BasicTxDetails>>(&path)
            } else {
                foundry_common::fs::read_json_file::<Vec<BasicTxDetails>>(&path)
            };

            let Ok(tx_seq) = read_corpus_result else {
                trace!(target: "corpus", "failed to load corpus from {}", path.display());
                continue
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
            corpus_dir: Some(corpus_dir),
            corpus_gzip,
            corpus_max_mutations,
            corpus_min_size,
            in_memory_corpus,
            current_mutated: None,
            failed_replays,
        })
    }

    /// Collects inputs from given invariant run, if new coverage produced.
    /// Persists call sequence (if corpus directory is configured) and updates in-memory corpus.
    pub fn collect_inputs(&mut self, test_run: &InvariantTestRun) {
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
        if let Some(corpus_dir) = &self.corpus_dir {
            let write_result = if self.corpus_gzip {
                foundry_common::fs::write_json_gzip_file(
                    corpus_dir.join(format!("{corpus_uuid}.gz")).as_path(),
                    &corpus.tx_seq,
                )
            } else {
                foundry_common::fs::write_json_file(
                    corpus_dir.join(format!("{corpus_uuid}.json")).as_path(),
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
        }

        // This includes reverting txs in the corpus and `can_continue` removes
        // them. We want this as it is new coverage and may help reach the other branch.
        self.in_memory_corpus.push(corpus);
    }

    /// Generates new call sequence from in memory corpus. Evicts oldest corpus mutated more than
    /// configured max mutations value.
    #[allow(clippy::needless_range_loop)]
    pub fn new_sequence(&mut self, test: &InvariantTest) -> eyre::Result<Vec<BasicTxDetails>> {
        let mut new_seq = vec![];
        let test_runner = &mut test.execution_data.borrow_mut().branch_runner;

        if !self.in_memory_corpus.is_empty() {
            let rng = test_runner.rng();

            // Flush oldest corpus mutated more than configured max mutations.
            let should_evict = self.in_memory_corpus.len() > self.corpus_min_size;
            if should_evict {
                if let Some(index) = self
                    .in_memory_corpus
                    .iter()
                    .position(|corpus| corpus.total_mutations > self.corpus_max_mutations)
                {
                    let corpus = self.in_memory_corpus.get(index).unwrap();
                    debug!(target: "corpus", "remove corpus {}", corpus.uuid);

                    // Flush to disk the seed metadata at the time of eviction.
                    if let Some(corpus_dir) = &self.corpus_dir {
                        let eviction_time = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_secs();
                        foundry_common::fs::write_json_file(
                            corpus_dir
                                .join(format!("{}-{}-metadata.json", corpus.uuid, eviction_time))
                                .as_path(),
                            &corpus,
                        )?
                    }
                    // Remove corpus from memory.
                    self.in_memory_corpus.remove(index);
                }
            }

            if self.in_memory_corpus.len() > 1 {
                let corpus_len = self.in_memory_corpus.len();
                let primary = &self.in_memory_corpus[rng.gen_range(0..corpus_len)];
                let secondary = &self.in_memory_corpus[rng.gen_range(0..corpus_len)];

                // TODO rounds of mutations on elements?
                match rng.gen_range(0..=5) {
                    // TODO expose config and add tests
                    // splice
                    0 => {
                        trace!(target: "corpus", "splice {} and {}", primary.uuid, secondary.uuid);
                        if should_evict {
                            self.current_mutated = Some(primary.uuid);
                        }
                        let start1 = rng.gen_range(0..primary.tx_seq.len());
                        let end1 = rng.gen_range(start1..primary.tx_seq.len());

                        let start2 = rng.gen_range(0..secondary.tx_seq.len());
                        let end2 = rng.gen_range(start2..secondary.tx_seq.len());

                        for tx in primary.tx_seq.iter().take(end1).skip(start1) {
                            new_seq.push(tx.clone());
                        }
                        for tx in secondary.tx_seq.iter().take(end2).skip(start2) {
                            new_seq.push(tx.clone());
                        }
                    }
                    // repeat
                    1 => {
                        let corpus = if rng.gen_bool(0.5) { primary } else { secondary };
                        trace!(target: "corpus", "repeat {}", corpus.uuid);
                        if should_evict {
                            self.current_mutated = Some(corpus.uuid);
                        }
                        new_seq = corpus.tx_seq.clone();
                        let start = rng.gen_range(0..corpus.tx_seq.len());
                        let end = rng.gen_range(start..corpus.tx_seq.len());
                        let item_idx = rng.gen_range(0..corpus.tx_seq.len());
                        let item = &corpus.tx_seq[item_idx];
                        for i in start..end {
                            new_seq[i] = item.clone();
                        }
                    }
                    // interleave
                    2 => {
                        trace!(target: "corpus", "interleave {} with {}", primary.uuid, secondary.uuid);
                        if should_evict {
                            self.current_mutated = Some(primary.uuid);
                        }
                        for (tx1, tx2) in primary.tx_seq.iter().zip(secondary.tx_seq.iter()) {
                            // chunks?
                            let tx = if rng.gen_bool(0.5) { tx1.clone() } else { tx2.clone() };
                            new_seq.push(tx);
                        }
                    }
                    // 3. Overwrite prefix with new sequence.
                    3 => {
                        let corpus = if rng.gen_bool(0.5) { primary } else { secondary };
                        trace!(target: "corpus", "overwrite prefix of {}", corpus.uuid);
                        if should_evict {
                            self.current_mutated = Some(corpus.uuid);
                        }
                        new_seq = corpus.tx_seq.clone();
                        for i in 0..rng.gen_range(0..=new_seq.len()) {
                            new_seq[i] = self.new_tx(test_runner)?;
                        }
                    }
                    // 4. Overwrite suffix with new sequence.
                    4 => {
                        let corpus = if rng.gen_bool(0.5) { primary } else { secondary };
                        trace!(target: "corpus", "overwrite suffix of {}", corpus.uuid);
                        if should_evict {
                            self.current_mutated = Some(corpus.uuid);
                        }
                        new_seq = corpus.tx_seq.clone();
                        for i in
                            new_seq.len() - rng.gen_range(0..new_seq.len())..corpus.tx_seq.len()
                        {
                            new_seq[i] = self.new_tx(test_runner)?;
                        }
                    }
                    // 5. Select idx to mutate and change its args according to its ABI.
                    5 => {
                        let targets = test.targeted_contracts.targets.lock();
                        let corpus = if rng.gen_bool(0.5) { primary } else { secondary };
                        trace!(target: "corpus", "ABI mutate args of {}", corpus.uuid);
                        if should_evict {
                            self.current_mutated = Some(corpus.uuid);
                        }
                        new_seq = corpus.tx_seq.clone();

                        let idx = rng.gen_range(0..new_seq.len());
                        let tx = new_seq.get_mut(idx).unwrap();
                        if let (_, Some(function)) = targets.fuzzed_artifacts(tx) {
                            tx.call_details.calldata =
                                fuzz_calldata(function.clone(), &FuzzFixtures::default())
                                    .new_tree(test_runner)
                                    .map_err(|_| eyre!("Could not generate case"))?
                                    .current();
                        }
                    }
                    _ => {
                        unreachable!()
                    }
                };

                trace!(target: "corpus", "new sequence generated {} from corpus {:?}", new_seq.len(), self.current_mutated);
            }
        }

        // Make sure sequence contains at least one tx to start fuzzing from.
        if new_seq.is_empty() {
            new_seq.push(self.new_tx(test_runner)?);
        }

        Ok(new_seq)
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
