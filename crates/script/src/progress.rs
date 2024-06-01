use crate::sequence::ScriptSequence;
use alloy_chains::Chain;
use alloy_primitives::B256;
use foundry_cli::init_progress;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use parking_lot::RwLock;
use std::{collections::HashMap, sync::Arc, time::Duration};
use yansi::Paint;

/// State of [ProgressBar]s displayed for the given [ScriptSequence].
#[derive(Debug)]
pub struct SequenceProgressState {
    /// The top spinner with containt of the format "Sequence #{id} on {network} | {status}""
    top_spinner: ProgressBar,
    /// Progress bar with the count of transactions.
    txs: ProgressBar,
    /// Progress var with the count of confirmed transactions.
    receipts: ProgressBar,
    /// Standalone spinners for pending transactions.
    tx_spinners: HashMap<B256, ProgressBar>,
    /// Copy of the main [MultiProgress] instance.
    multi: MultiProgress,
}

impl SequenceProgressState {
    pub fn new(sequence_idx: usize, sequence: &ScriptSequence, multi: MultiProgress) -> Self {
        let mut template = "{spinner:.green}".to_string();
        template.push_str(
            format!(" Sequence #{} on {}", sequence_idx + 1, Chain::from(sequence.chain)).as_str(),
        );
        template.push_str("{msg}");

        let top_spinner = ProgressBar::new_spinner()
            .with_style(ProgressStyle::with_template(&template).unwrap().tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈✅"));
        let top_spinner = multi.add(top_spinner);

        let txs = multi.insert_after(
            &top_spinner,
            init_progress!(sequence.transactions, "txes").with_prefix("    "),
        );

        let receipts = multi.insert_after(
            &txs,
            init_progress!(sequence.transactions, "receipts").with_prefix("    "),
        );

        top_spinner.enable_steady_tick(Duration::from_millis(100));
        txs.enable_steady_tick(Duration::from_millis(1000));
        receipts.enable_steady_tick(Duration::from_millis(1000));

        txs.set_position(sequence.receipts.len() as u64);
        receipts.set_position(sequence.receipts.len() as u64);

        let mut state = SequenceProgressState {
            top_spinner,
            txs,
            receipts,
            tx_spinners: Default::default(),
            multi,
        };

        for tx_hash in sequence.pending.iter() {
            state.tx_sent(*tx_hash);
        }

        state
    }

    /// Called when a new transaction is sent. Displays a spinner with a hash of the transaction and
    /// advances the sent transactions progress bar.
    pub fn tx_sent(&mut self, tx_hash: B256) {
        // Avoid showing more than 10 spinners.
        if self.tx_spinners.len() < 10 {
            let spinner = ProgressBar::new_spinner()
                .with_style(
                    ProgressStyle::with_template("    {spinner:.green} {msg}")
                        .unwrap()
                        .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈"),
                )
                .with_message(format!("{} {}", "[Pending]".yellow(), tx_hash));

            let spinner = self.multi.insert_before(&self.txs, spinner);
            spinner.enable_steady_tick(Duration::from_millis(100));

            self.tx_spinners.insert(tx_hash, spinner);
        }
        self.txs.inc(1);
    }

    /// Removes the pending transaction spinner and advances confirmed transactions progress bar.
    pub fn finish_tx_spinner(&mut self, tx_hash: B256) {
        if let Some(spinner) = self.tx_spinners.remove(&tx_hash) {
            spinner.finish_and_clear();
        }
        self.receipts.inc(1);
    }

    /// Same as finish_tx_spinner but also prints a message to stdout above all other progress bars.
    pub fn finish_tx_spinner_with_msg(&mut self, tx_hash: B256, msg: &str) -> std::io::Result<()> {
        self.finish_tx_spinner(tx_hash);
        self.multi.println(msg)?;

        Ok(())
    }

    /// Sets status for the current sequence progress.
    pub fn set_status(&mut self, status: &str) {
        self.top_spinner.set_message(format!(" | {status}"));
    }

    /// Hides transactions and receipts progress bar, leaving only top line with the latest set
    /// status.
    pub fn finish(&self) {
        self.top_spinner.finish();
        self.txs.finish_and_clear();
        self.receipts.finish_and_clear();
    }
}

/// Clonable wrapper around [SequenceProgressState].
#[derive(Debug, Clone)]
pub struct SequenceProgress {
    pub inner: Arc<RwLock<SequenceProgressState>>,
}

impl SequenceProgress {
    pub fn new(sequence_idx: usize, sequence: &ScriptSequence, multi: MultiProgress) -> Self {
        Self {
            inner: Arc::new(RwLock::new(SequenceProgressState::new(sequence_idx, sequence, multi))),
        }
    }
}

/// Container for multiple [SequenceProgress] instances keyed by sequence index.
#[derive(Debug, Clone, Default)]
pub struct ScriptProgress {
    state: Arc<RwLock<HashMap<usize, SequenceProgress>>>,
    multi: MultiProgress,
}

impl ScriptProgress {
    /// Returns a [SequenceProgress] instance for the given sequence index. If it doesn't exist,
    /// creates one.
    pub fn get_sequence_progress(
        &self,
        sequence_idx: usize,
        sequence: &ScriptSequence,
    ) -> SequenceProgress {
        if let Some(progress) = self.state.read().get(&sequence_idx) {
            return progress.clone();
        }
        let progress = SequenceProgress::new(sequence_idx, sequence, self.multi.clone());
        self.state.write().insert(sequence_idx, progress.clone());
        progress
    }
}
