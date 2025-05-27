use crate::executors::{invariant::InvariantTestRun, Executor};
use alloy_primitives::U256;
use eyre::{ensure, eyre};
use foundry_evm_fuzz::invariant::BasicTxDetails;
use proptest::{prelude::Rng, test_runner::TestRunner};
use std::{path::PathBuf, time};

/// Invariant corpus manager.
#[derive(Default)]
pub struct TxCorpusManager {
    // Path to invariant corpus directory. If None, corpus with new coverage is not persisted.
    pub corpus_dir: Option<PathBuf>,
    // In-memory corpus, populated from persisted files and current runs.
    pub in_memory_corpus: Vec<Vec<BasicTxDetails>>, /* TODO need some sort of corpus management
                                                     * (limit memory usage and flush). */
}

impl TxCorpusManager {
    pub fn new(
        corpus_dir: &Option<PathBuf>,
        test_name: &String,
        executor: &Executor,
        history_map: &mut [u8],
    ) -> eyre::Result<Self> {
        // Early return if corpus dir not configured.
        let Some(corpus_dir) = corpus_dir else { return Ok(Self::default()) };

        // Ensure corpus dir for invariant function is created.
        let corpus_dir = corpus_dir.join(test_name);
        if !corpus_dir.is_dir() {
            foundry_common::fs::create_dir_all(&corpus_dir)?;
        }

        // Sort by filename (milliseconds since epoch) for loading corpus files in proper sequence.
        let mut entries: Vec<_> =
            std::fs::read_dir(&corpus_dir)?.filter_map(|entry| entry.ok()).collect();
        entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_string());

        let mut in_memory_corpus = vec![];
        // Load sequences from each corpus file, replay calls to warm up history map and populate
        // in memory corpus.
        for entry in entries {
            let path = entry.path();
            let tx_seq: Vec<BasicTxDetails> =
                foundry_common::fs::read_json_file(&path).expect("msg");

            if !tx_seq.is_empty() {
                // Warm up history map from loaded sequences.
                let mut executor = executor.clone();
                let mut new_coverage = false;
                for tx in &tx_seq {
                    let mut call_result = executor
                        .call_raw(
                            tx.sender,
                            tx.call_details.target,
                            tx.call_details.calldata.clone(),
                            U256::ZERO,
                        )
                        .map_err(|e| eyre!(format!("Could not make raw evm call: {e}")))?;
                    if call_result.merge_edge_coverage(history_map) {
                        new_coverage = true;
                    }
                    executor.commit(&mut call_result);
                }
                ensure!(
                    new_coverage,
                    "loaded corpus from {} does not result in new coverage",
                    path.display()
                );
                trace!(
                    target: "corpus",
                    "load sequence with len {} from corpus file {}",
                    tx_seq.len(),
                    path.display()
                );

                // Populate in memory corpus with sequence from corpus file.
                in_memory_corpus.push(tx_seq);
            }
        }

        Ok(Self { corpus_dir: Some(corpus_dir), in_memory_corpus })
    }

    /// Collects inputs from given invariant run, if new coverage produced.
    /// Persists call sequence (if corpus directory is configured) and updates in-memory corpus.
    pub fn collect_inputs(&mut self, test_run: &InvariantTestRun) {
        // Collect inputs only if current run produced new coverage.
        if !test_run.new_coverage {
            return;
        }

        let inputs = test_run.inputs.clone();

        // Persist to disk if corpus dir is configured.
        if let Some(corpus_dir) = &self.corpus_dir {
            let timestamp = time::SystemTime::now()
                .duration_since(time::UNIX_EPOCH)
                .expect("Time went backwards")
                .as_micros()
                .to_string();
            let path = corpus_dir.join(timestamp);
            trace!(
                target: "corpus",
                "persist inputs {} for new coverage in corpus file {}",
                inputs.len(),
                path.display()
            );

            if let Err(err) = foundry_common::fs::write_json_file(path.as_path(), &inputs) {
                error!(%err, "Failed to record call sequence");
            }
        }

        // This includes reverting txs in the corpus and `can_continue` removes
        // them. We want this as it is new coverage and may help reach the other branch.
        self.in_memory_corpus.push(inputs);
    }

    /// Generates new call sequence from in memory corpus.
    #[allow(clippy::needless_range_loop)]
    pub fn new_sequence(&self, test_runnner: &mut TestRunner) -> Vec<BasicTxDetails> {
        let mut new_seq = vec![];
        let rng = test_runnner.rng();

        if self.in_memory_corpus.len() > 1 {
            let idx1 = rng.gen_range(0..self.in_memory_corpus.len());
            let idx2 = rng.gen_range(0..self.in_memory_corpus.len());
            let one = &self.in_memory_corpus[idx1];
            let two = &self.in_memory_corpus[idx2];
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
        }
        trace!(target: "corpus", "new sequence generated {}", new_seq.len());
        new_seq
    }
}
