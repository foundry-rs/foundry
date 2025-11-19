use crate::receipts::{PendingReceiptError, TxStatus, check_tx_status, format_receipt};
use alloy_chains::Chain;
use alloy_primitives::{
    B256,
    map::{B256HashMap, HashMap},
};
use eyre::Result;
use forge_script_sequence::ScriptSequence;
use foundry_cli::utils::init_progress;
use foundry_common::{provider::RetryProvider, shell};
use futures::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use parking_lot::RwLock;
use std::{fmt::Write, sync::Arc, time::Duration};
use yansi::Paint;

/// State of [ProgressBar]s displayed for the given [ScriptSequence].
#[derive(Debug)]
pub struct SequenceProgressState {
    /// The top spinner with content of the format "Sequence #{id} on {network} | {status}""
    top_spinner: ProgressBar,
    /// Progress bar with the count of transactions.
    txs: ProgressBar,
    /// Progress var with the count of confirmed transactions.
    receipts: ProgressBar,
    /// Standalone spinners for pending transactions.
    tx_spinners: B256HashMap<ProgressBar>,
    /// Copy of the main [MultiProgress] instance.
    multi: MultiProgress,
}

impl SequenceProgressState {
    pub fn new(sequence_idx: usize, sequence: &ScriptSequence, multi: MultiProgress) -> Self {
        let mut state = if shell::is_quiet() || shell::is_json() {
            let top_spinner = ProgressBar::hidden();
            let txs = ProgressBar::hidden();
            let receipts = ProgressBar::hidden();

            Self { top_spinner, txs, receipts, tx_spinners: Default::default(), multi }
        } else {
            let mut template = "{spinner:.green}".to_string();
            write!(template, " Sequence #{} on {}", sequence_idx + 1, Chain::from(sequence.chain))
                .unwrap();
            template.push_str("{msg}");

            let top_spinner = ProgressBar::new_spinner().with_style(
                ProgressStyle::with_template(&template).unwrap().tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈✅"),
            );
            let top_spinner = multi.add(top_spinner);

            let txs = multi.insert_after(
                &top_spinner,
                init_progress(sequence.transactions.len() as u64, "txes").with_prefix("    "),
            );

            let receipts = multi.insert_after(
                &txs,
                init_progress(sequence.transactions.len() as u64, "receipts").with_prefix("    "),
            );

            top_spinner.enable_steady_tick(Duration::from_millis(100));
            txs.enable_steady_tick(Duration::from_millis(1000));
            receipts.enable_steady_tick(Duration::from_millis(1000));

            txs.set_position(sequence.receipts.len() as u64);
            receipts.set_position(sequence.receipts.len() as u64);

            Self { top_spinner, txs, receipts, tx_spinners: Default::default(), multi }
        };

        for tx_hash in &sequence.pending {
            state.tx_sent(*tx_hash);
        }

        state
    }

    /// Called when a new transaction is sent. Displays a spinner with a hash of the transaction and
    /// advances the sent transactions progress bar.
    pub fn tx_sent(&mut self, tx_hash: B256) {
        // Avoid showing more than 10 spinners.
        if self.tx_spinners.len() < 10 {
            let spinner = if shell::is_quiet() || shell::is_json() {
                ProgressBar::hidden()
            } else {
                let spinner = ProgressBar::new_spinner()
                    .with_style(
                        ProgressStyle::with_template("    {spinner:.green} {msg}")
                            .unwrap()
                            .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈"),
                    )
                    .with_message(format!("{} {}", "[Pending]".yellow(), tx_hash));

                let spinner = self.multi.insert_before(&self.txs, spinner);
                spinner.enable_steady_tick(Duration::from_millis(100));
                spinner
            };

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

        if !(shell::is_quiet() || shell::is_json()) {
            self.multi.println(msg)?;
        }

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

/// Cloneable wrapper around [SequenceProgressState].
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

    /// Traverses a set of pending transactions and either finds receipts, or clears
    /// them from the deployment sequence.
    ///
    /// For each `tx_hash`, we check if it has confirmed. If it has
    /// confirmed, we push the receipt (if successful) or push an error (if
    /// revert). If the transaction has not confirmed, but can be found in the
    /// node's mempool, we wait for its receipt to be available. If the transaction
    /// has not confirmed, and cannot be found in the mempool, we remove it from
    /// the `deploy_sequence.pending` vector so that it will be rebroadcast in
    /// later steps.
    pub async fn wait_for_pending(
        &self,
        sequence_idx: usize,
        deployment_sequence: &mut ScriptSequence,
        provider: &RetryProvider,
        timeout: u64,
    ) -> Result<()> {
        if deployment_sequence.pending.is_empty() {
            return Ok(());
        }

        let count = deployment_sequence.pending.len();
        let seq_progress = self.get_sequence_progress(sequence_idx, deployment_sequence);

        seq_progress.inner.write().set_status("Waiting for pending transactions");

        trace!("Checking status of {count} pending transactions");

        let futs = deployment_sequence
            .pending
            .clone()
            .into_iter()
            .map(|tx| check_tx_status(provider, tx, timeout));
        let mut tasks = futures::stream::iter(futs).buffer_unordered(10);

        let mut errors: Vec<String> = vec![];
        let mut discarded_transactions = false;

        while let Some((tx_hash, result)) = tasks.next().await {
            match result {
                Err(err) => {
                    // Check if this is a retry error for pending receipts
                    if err.downcast_ref::<PendingReceiptError>().is_some() {
                        // We've already retried several times with sleep, but the receipt is still
                        // pending
                        discarded_transactions = true;
                        deployment_sequence.remove_pending(tx_hash);
                        seq_progress
                            .inner
                            .write()
                            .finish_tx_spinner_with_msg(tx_hash, &err.to_string())?;
                    } else {
                        errors.push(format!(
                            "Failure on receiving a receipt for {tx_hash:?}:\n{err}"
                        ));
                        seq_progress.inner.write().finish_tx_spinner(tx_hash);
                    }
                }
                Ok(TxStatus::Dropped) => {
                    // We want to remove it from pending so it will be re-broadcast.
                    deployment_sequence.remove_pending(tx_hash);
                    discarded_transactions = true;

                    let msg = format!(
                        "Transaction {tx_hash:?} dropped from the mempool. It will be retried when using --resume."
                    );
                    seq_progress.inner.write().finish_tx_spinner_with_msg(tx_hash, &msg)?;
                }
                Ok(TxStatus::Success(receipt)) => {
                    trace!(tx_hash=?tx_hash, "received tx receipt");

                    let msg = format_receipt(deployment_sequence.chain.into(), &receipt);
                    seq_progress.inner.write().finish_tx_spinner_with_msg(tx_hash, &msg)?;

                    deployment_sequence.remove_pending(receipt.transaction_hash);
                    deployment_sequence.add_receipt(receipt);
                }
                Ok(TxStatus::Revert(receipt)) => {
                    // consider:
                    // if this is not removed from pending, then the script becomes
                    // un-resumable. Is this desirable on reverts?
                    warn!(tx_hash=?tx_hash, "Transaction Failure");
                    deployment_sequence.remove_pending(receipt.transaction_hash);

                    let msg = format_receipt(deployment_sequence.chain.into(), &receipt);
                    seq_progress.inner.write().finish_tx_spinner_with_msg(tx_hash, &msg)?;

                    errors.push(format!("Transaction Failure: {:?}", receipt.transaction_hash));
                }
            }
        }

        // print any errors
        if !errors.is_empty() {
            let mut error_msg = errors.join("\n");

            // Add information about using --resume if necessary
            if !deployment_sequence.pending.is_empty() || discarded_transactions {
                error_msg += r#"

Add `--resume` to your command to try and continue broadcasting the transactions. This will attempt to resend transactions that were discarded by the RPC."#;
            }

            eyre::bail!(error_msg);
        } else if discarded_transactions {
            // If we have discarded transactions but no errors, still inform the user
            sh_warn!(
                "Some transactions were discarded by the RPC node. Use `--resume` to retry these transactions."
            )?;
        }

        Ok(())
    }
}
