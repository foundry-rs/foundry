use crate::executors::{invariant::InvariantTestRun, Executor};
use alloy_primitives::U256;
use eyre::eyre;
use foundry_config::InvariantConfig;
use foundry_evm_fuzz::invariant::BasicTxDetails;
use proptest::{prelude::Rng, test_runner::TestRunner};
use std::path::PathBuf;
use uuid::Uuid;

type Corpus = (Vec<BasicTxDetails>, CorpusMetadata);

/// Corpus metadata.
#[derive(Debug)]
struct CorpusMetadata {
    // Unique corpus identifier.
    uuid: Uuid,
    // Total mutations of corpus as primary source.
    total_mutations: usize,
    // New coverage explored by mutating corpus.
    new_coverage: usize,
}

impl CorpusMetadata {
    pub fn new(uuid: Uuid) -> Self {
        Self { uuid, total_mutations: 0, new_coverage: 0 }
    }
}

/// Invariant corpus manager.
#[derive(Default)]
pub struct TxCorpusManager {
    // Path to invariant corpus directory. If None, corpus with new coverage is not persisted.
    corpus_dir: Option<PathBuf>,
    // Whether corpus to use gzip file compression and decompression.
    corpus_gzip: bool,
    // Number of corpus mutations until marked as eligible to be flushed from in-memory corpus.
    corpus_max_mutations: usize,
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
        executor: &Executor,
        history_map: &mut [u8],
    ) -> eyre::Result<Self> {
        // Early return if corpus dir not configured.
        let Some(corpus_dir) = &invariant_config.corpus_dir else { return Ok(Self::default()) };

        // Ensure corpus dir for invariant function is created.
        let corpus_dir = corpus_dir.join(test_name);
        if !corpus_dir.is_dir() {
            foundry_common::fs::create_dir_all(&corpus_dir)?;
        }

        let mut in_memory_corpus = vec![];
        let mut failed_replays = 0;
        let corpus_gzip = invariant_config.corpus_gzip;
        let corpus_max_mutations = invariant_config.corpus_max_mutations;
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
                    if !call_result.reverted {
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
                let uuid = if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    Uuid::try_from(stem.to_string())?
                } else {
                    Uuid::new_v4()
                };
                in_memory_corpus.push((tx_seq, CorpusMetadata::new(uuid)));
            }
        }

        Ok(Self {
            corpus_dir: Some(corpus_dir),
            corpus_gzip,
            corpus_max_mutations,
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
                self.in_memory_corpus.iter_mut().find(|corpus| corpus.1.uuid.eq(uuid))
            {
                corpus.1.total_mutations += 1;
                if test_run.new_coverage {
                    corpus.1.new_coverage += 1
                }

                trace!(
                    target: "corpus",
                    "updated corpus stats {:?} corpus",
                    corpus.1
                );
            }

            self.current_mutated = None;
        }

        // Collect inputs only if current run produced new coverage.
        if !test_run.new_coverage {
            return;
        }

        let inputs = test_run.inputs.clone();
        let corpus_uuid = Uuid::new_v4();

        // Persist to disk if corpus dir is configured.
        if let Some(corpus_dir) = &self.corpus_dir {
            let write_result = if self.corpus_gzip {
                foundry_common::fs::write_json_gzip_file(
                    corpus_dir.join(format!("{corpus_uuid}.gz")).as_path(),
                    &inputs,
                )
            } else {
                foundry_common::fs::write_json_file(
                    corpus_dir.join(format!("{corpus_uuid}.json")).as_path(),
                    &inputs,
                )
            };

            if let Err(err) = write_result {
                debug!(target: "corpus", %err, "Failed to record call sequence {:?}", inputs);
            } else {
                trace!(
                    target: "corpus",
                    "persisted {} inputs for new coverage in {corpus_uuid} corpus",
                    inputs.len()
                );
            }
        }

        // This includes reverting txs in the corpus and `can_continue` removes
        // them. We want this as it is new coverage and may help reach the other branch.
        self.in_memory_corpus.push((inputs, CorpusMetadata::new(corpus_uuid)));
    }

    /// Generates new call sequence from in memory corpus. Evicts oldest corpus mutated more than
    /// configured max mutations value.
    #[allow(clippy::needless_range_loop)]
    pub fn new_sequence(&mut self, test_runnner: &mut TestRunner) -> Vec<BasicTxDetails> {
        let mut new_seq = vec![];
        let rng = test_runnner.rng();

        // Flush oldest corpus mutated more than configured max mutations.
        if let Some(index) = self
            .in_memory_corpus
            .iter()
            .position(|corpus| corpus.1.total_mutations > self.corpus_max_mutations)
        {
            let uuid = self.in_memory_corpus.get(index).unwrap().1.uuid;
            debug!(target: "corpus", "remove corpus with {uuid}");
            self.in_memory_corpus.remove(index);
        }

        if self.in_memory_corpus.len() > 1 {
            let idx1 = rng.gen_range(0..self.in_memory_corpus.len());
            let idx2 = rng.gen_range(0..self.in_memory_corpus.len());

            let primary_corpus = &self.in_memory_corpus[idx1];

            let one = &primary_corpus.0;
            let two = &self.in_memory_corpus[idx2].0;
            // TODO rounds of mutations on elements?
            match rng.gen_range(0..3) {
                // TODO expose config and add tests
                // splice
                0 => {
                    let start1 = rng.gen_range(0..one.len());
                    let end1 = rng.gen_range(start1..one.len());

                    let start2 = rng.gen_range(0..two.len());
                    let end2 = rng.gen_range(start2..two.len());

                    for tx in one.iter().take(end1).skip(start1) {
                        new_seq.push(tx.clone());
                    }

                    for tx in two.iter().take(end2).skip(start2) {
                        new_seq.push(tx.clone());
                    }
                }
                // repeat
                1 => {
                    let tx = if rng.gen_bool(0.5) { one } else { two };
                    new_seq = tx.clone();
                    let start = rng.gen_range(0..tx.len());
                    let end = rng.gen_range(start..tx.len());
                    let item_idx = rng.gen_range(0..tx.len());
                    let item = &tx[item_idx];
                    for i in start..end {
                        new_seq[i] = item.clone();
                    }
                }
                // interleave
                2 => {
                    for (tx1, tx2) in one.iter().zip(two.iter()) {
                        // chunks?
                        let tx = if rng.gen_bool(0.5) { tx1.clone() } else { tx2.clone() };
                        new_seq.push(tx);
                    }
                }
                // TODO
                // 3. Overwrite prefix with new or mutated sequence
                // 4. Overwrite suffix with new or mutated sequence
                // 5. Select idx to mutate and change its args according to its ABI
                _ => {
                    unreachable!();
                }
            }

            // Record corpus uuid if mutated corpus results in non-empty new sequence.
            if !new_seq.is_empty() {
                self.current_mutated = Some(primary_corpus.1.uuid);
            }
            trace!(target: "corpus", "new sequence generated {} from corpus {:?}", new_seq.len(), self.current_mutated);
        }

        new_seq
    }

    pub fn failed_replays(self) -> usize {
        self.failed_replays
    }
}
