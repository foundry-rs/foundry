use crate::receipts::{PendingReceiptError, TxStatus, check_tx_status, format_receipt};
use alloy_chains::Chain;
use alloy_network::{Network, ReceiptResponse, TransactionResponse};
use alloy_primitives::{
    Address, B256,
    map::{B256HashMap, HashMap},
};
use alloy_provider::{Provider, RootProvider};
use eyre::Result;
use forge_script_sequence::ScriptSequence;
use foundry_cli::utils::init_progress;
use foundry_common::shell;
use futures::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use parking_lot::RwLock;
use std::{fmt::Write, sync::Arc, time::Duration};
use yansi::Paint;

/// Sender, nonce and latest known submission of an unreceipted operation.
pub(crate) type PredecessorOperation = (Address, u64, Option<B256>);

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
    pub fn new<N: Network>(
        sequence_idx: usize,
        sequence: &ScriptSequence<N>,
        multi: MultiProgress,
    ) -> Self {
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
    pub fn new<N: Network>(
        sequence_idx: usize,
        sequence: &ScriptSequence<N>,
        multi: MultiProgress,
    ) -> Self {
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
    pub fn get_sequence_progress<N: Network>(
        &self,
        sequence_idx: usize,
        sequence: &ScriptSequence<N>,
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
    pub async fn wait_for_pending<N: Network>(
        &self,
        sequence_idx: usize,
        deployment_sequence: &mut ScriptSequence<N>,
        provider: &RootProvider<N>,
        timeout: u64,
        confirmations: u64,
        submission_hashes: (&[B256], &[B256], &[PredecessorOperation]),
    ) -> Result<()> {
        let (durable_hashes, replayable_hashes, operation_hashes) = submission_hashes;
        if deployment_sequence.pending.is_empty() {
            return Ok(());
        }

        let count = deployment_sequence.pending.len();
        let seq_progress = self.get_sequence_progress(sequence_idx, deployment_sequence);

        seq_progress.inner.write().set_status("Waiting for pending transactions");

        trace!("Checking status of {count} pending transactions");

        let mut waits = deployment_sequence
            .pending
            .iter()
            .map(|&tx| (tx, predecessors(operation_hashes, tx)))
            .filter(|(_, predecessors)| !predecessors.operations.is_empty())
            .collect::<Vec<_>>();
        let futs = deployment_sequence
            .pending
            .clone()
            .into_iter()
            .map(|tx| check_tx_status(provider, tx, timeout, confirmations));
        let mut tasks = futures::stream::iter(futs).buffer_unordered(10);

        let mut errors: Vec<String> = vec![];
        let mut discarded_transactions = false;

        loop {
            let (tx_hash, result) = tokio::select! {
                next = tasks.next() => {
                    let Some(next) = next else { break };
                    next
                }
                // RPC calls may reach different nodes, so a gap is advisory. Keep reconciling
                // receipts and dropped transactions, and warn only once for each successor.
                (tx_hash, warning) = blocked_transaction(provider, &waits, timeout) => {
                    sh_warn!("{warning}")?;
                    waits.retain(|(hash, _)| *hash != tx_hash);
                    continue;
                }
            };
            waits.retain(|(hash, _)| *hash != tx_hash);
            match result {
                Err(err) => {
                    // Check if this is a retry error for pending receipts
                    if err.downcast_ref::<PendingReceiptError>().is_some() {
                        // We've already retried several times with sleep, but the receipt is still
                        // pending
                        if durable_hashes.contains(&tx_hash) {
                            errors.push(format!(
                                "Durable submission {tx_hash:?} is still pending; refusing to discard its recovery identity"
                            ));
                        } else {
                            discarded_transactions = true;
                            deployment_sequence.remove_pending(tx_hash);
                        }
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
                    if replayable_hashes.contains(&tx_hash) {
                        deployment_sequence.remove_pending(tx_hash);
                        discarded_transactions = true;
                    } else if durable_hashes.contains(&tx_hash) {
                        errors.push(format!(
                            "Durable submission {tx_hash:?} is not currently visible; refusing to discard its recovery identity"
                        ));
                    } else {
                        // We want to remove it from pending so it will be re-broadcast.
                        deployment_sequence.remove_pending(tx_hash);
                        discarded_transactions = true;
                    }

                    let msg = format!(
                        "Transaction {tx_hash:?} is not currently visible to the RPC endpoint."
                    );
                    seq_progress.inner.write().finish_tx_spinner_with_msg(tx_hash, &msg)?;
                }
                Ok(TxStatus::Success(receipt)) => {
                    trace!(tx_hash=?tx_hash, "received tx receipt");

                    let msg = format_receipt(
                        deployment_sequence.chain.into(),
                        &receipt,
                        Some(deployment_sequence),
                    );
                    seq_progress.inner.write().finish_tx_spinner_with_msg(tx_hash, &msg)?;

                    deployment_sequence.remove_pending(receipt.transaction_hash());
                    deployment_sequence.add_receipt(receipt);
                }
                Ok(TxStatus::Revert(receipt)) => {
                    // consider:
                    // if this is not removed from pending, then the script becomes
                    // un-resumable. Is this desirable on reverts?
                    warn!(tx_hash=?tx_hash, "Transaction Failure");
                    deployment_sequence.remove_pending(receipt.transaction_hash());

                    let msg = format_receipt(
                        deployment_sequence.chain.into(),
                        &receipt,
                        Some(deployment_sequence),
                    );
                    seq_progress.inner.write().finish_tx_spinner_with_msg(tx_hash, &msg)?;

                    errors.push(format!("Transaction Failure: {:?}", receipt.transaction_hash()));
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

/// Earlier operations from the sender of a pending transaction that have no receipt.
#[derive(Debug, Default)]
struct Predecessors {
    sender: Address,
    /// Nonce and latest known submission hash of each operation.
    operations: Vec<(u64, Option<B256>)>,
}

/// Collects lower-nonce operations from the same RPC and sender as the pending submission.
fn predecessors(operations: &[PredecessorOperation], hash: B256) -> Predecessors {
    let Some(&(sender, nonce, _)) = operations.iter().find(|(_, _, saved)| *saved == Some(hash))
    else {
        return Predecessors::default();
    };
    let operations = operations
        .iter()
        .filter_map(|&(from, earlier, hash)| {
            (from == sender && earlier < nonce).then_some((earlier, hash))
        })
        .collect();
    Predecessors { sender, operations }
}

/// Reports an apparent nonce gap only while the successor remains visible and pending.
/// The evidence is advisory because a load-balanced RPC can return inconsistent views.
///
/// Each check runs after a full receipt-watcher timeout, matching the evidence used to treat a
/// transaction as dropped.
async fn blocked_transaction<N: Network>(
    provider: &RootProvider<N>,
    waits: &[(B256, Predecessors)],
    timeout: u64,
) -> (B256, String) {
    loop {
        tokio::time::sleep(Duration::from_secs(timeout.max(1))).await;
        for (hash, predecessors) in waits {
            if let Some(nonce) = blocked_nonce(provider, *hash, predecessors).await {
                return (
                    *hash,
                    format!(
                        "transaction {hash} appears to be blocked: the RPC endpoint reports nonce {nonce} from {} as unfilled and does not return its saved transaction. Check the predecessor transaction and the RPC endpoint before retrying",
                        predecessors.sender
                    ),
                );
            }
        }
    }
}

/// Requires the successor to still be visible and unmined before reporting a nonce gap.
async fn blocked_nonce<N: Network>(
    provider: &RootProvider<N>,
    hash: B256,
    predecessors: &Predecessors,
) -> Option<u64> {
    let nonce = missing_predecessor(provider, predecessors).await?;
    matches!(provider.get_transaction_by_hash(hash).await, Ok(Some(tx)) if tx.block_number().is_none())
        .then_some(nonce)
}

/// Returns the predecessor nonce that the node reports as the sender's next unfilled nonce, when
/// the script's known submission for it is not returned by the node.
///
/// Nonces above the pending nonce can still be filled by queued transactions, including
/// replacements, so only the operation at the pending nonce is checked. The pending nonce is read
/// again after the lookup, since the node can fill or open nonces in between. A gap outside the
/// script's operations, a predecessor without a known hash, or a failed lookup is not evidence that
/// the script must resubmit anything.
async fn missing_predecessor<N: Network>(
    provider: &RootProvider<N>,
    predecessors: &Predecessors,
) -> Option<u64> {
    let pending_nonce =
        async || provider.get_transaction_count(predecessors.sender).pending().await.ok();
    let unfilled = pending_nonce().await?;
    let hash = predecessors
        .operations
        .iter()
        .find_map(|&(nonce, hash)| (nonce == unfilled).then_some(hash))??;
    if !matches!(provider.get_transaction_by_hash(hash).await, Ok(None)) {
        return None;
    }
    (pending_nonce().await? == unfilled).then_some(unfilled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_network::{Ethereum, TransactionBuilder};
    use alloy_primitives::{U64, U256};
    use alloy_provider::{ProviderBuilder, mock::Asserter};
    use alloy_rpc_types::TransactionRequest;

    async fn send(provider: &impl Provider, from: Address, nonce: u64) -> B256 {
        let tx = TransactionRequest::default()
            .with_from(from)
            .with_to(from)
            .with_value(U256::from(1))
            .with_nonce(nonce);
        *provider.send_transaction(tx).await.unwrap().tx_hash()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn missing_predecessor_requires_known_invisible_unmined_submission() {
        let (api, handle) = anvil::spawn(anvil::NodeConfig::test().with_no_mining(true)).await;
        let provider = ProviderBuilder::new()
            .connect_http(handle.http_endpoint().parse().unwrap())
            .root()
            .clone();
        let sender = handle.dev_accounts().next().unwrap();
        let unknown = B256::repeat_byte(0xab);
        let missing = |operations| async {
            missing_predecessor::<Ethereum>(&provider, &Predecessors { sender, operations }).await
        };

        // A known submission the node does not return at the first unfilled nonce blocks its
        // successors.
        assert_eq!(missing(vec![(0, Some(unknown))]).await, Some(0));
        // An unknown submission hash at that nonce is not evidence that it is absent, and later
        // nonces may be filled by queued transactions.
        assert_eq!(missing(vec![(0, None)]).await, None);
        assert_eq!(missing(vec![(0, None), (1, Some(unknown))]).await, None);

        // A visible predecessor can still be mined.
        let visible = send(&provider, sender, 0).await;
        assert_eq!(missing(vec![(0, Some(visible))]).await, None);

        // A mined predecessor is skipped even when its recorded hash is not returned.
        api.mine_one().await.unwrap();
        assert_eq!(missing(vec![(0, Some(unknown))]).await, None);
    }

    /// A fee replacement fills the nonce under a new hash, so the successor can still be mined.
    #[tokio::test(flavor = "multi_thread")]
    async fn missing_predecessor_accepts_replaced_submission() {
        let (api, handle) = anvil::spawn(anvil::NodeConfig::test().with_no_mining(true)).await;
        let provider = ProviderBuilder::new()
            .connect_http(handle.http_endpoint().parse().unwrap())
            .root()
            .clone();
        let sender = handle.dev_accounts().next().unwrap();
        let submit = async |nonce: u64, fee_multiplier: u128| {
            let tx = TransactionRequest::default()
                .with_from(sender)
                .with_to(sender)
                .with_value(U256::from(1))
                .with_nonce(nonce)
                .with_max_fee_per_gas(100_000_000_000 * fee_multiplier)
                .with_max_priority_fee_per_gas(1_000_000_000 * fee_multiplier);
            *provider.send_transaction(tx).await.unwrap().tx_hash()
        };
        let original = submit(0, 1).await;
        let successor = submit(1, 1).await;
        submit(0, 2).await;
        assert!(provider.get_transaction_by_hash(original).await.unwrap().is_none());

        let predecessors = Predecessors { sender, operations: vec![(0, Some(original))] };
        assert_eq!(missing_predecessor::<Ethereum>(&provider, &predecessors).await, None);
        api.mine_one().await.unwrap();
        assert!(provider.get_transaction_receipt(successor).await.unwrap().is_some());
    }

    /// A replacement can also fill a queued nonce above an unrelated gap, so a missing original
    /// hash there does not mean the nonce needs resubmission.
    #[tokio::test(flavor = "multi_thread")]
    async fn missing_predecessor_accepts_replaced_queued_submission() {
        let (api, handle) = anvil::spawn(anvil::NodeConfig::test().with_no_mining(true)).await;
        let provider = ProviderBuilder::new()
            .connect_http(handle.http_endpoint().parse().unwrap())
            .root()
            .clone();
        let sender = handle.dev_accounts().next().unwrap();
        let submit = async |nonce: u64, fee_multiplier: u128| {
            let tx = TransactionRequest::default()
                .with_from(sender)
                .with_to(sender)
                .with_value(U256::from(1))
                .with_nonce(nonce)
                .with_max_fee_per_gas(100_000_000_000 * fee_multiplier)
                .with_max_priority_fee_per_gas(1_000_000_000 * fee_multiplier);
            *provider.send_transaction(tx).await.unwrap().tx_hash()
        };
        // Nonce 0 is outside the script; its operations start at nonce 1.
        let original = submit(1, 1).await;
        let successor = submit(2, 1).await;
        submit(1, 2).await;
        assert!(provider.get_transaction_by_hash(original).await.unwrap().is_none());

        let predecessors = Predecessors { sender, operations: vec![(1, Some(original))] };
        assert_eq!(missing_predecessor::<Ethereum>(&provider, &predecessors).await, None);
        submit(0, 1).await;
        api.mine_one().await.unwrap();
        assert!(provider.get_transaction_receipt(successor).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn missing_predecessor_checks_only_a_stable_pending_nonce() {
        let asserter = Asserter::new();
        let provider =
            ProviderBuilder::<_, _, Ethereum>::default().connect_mocked_client(asserter.clone());
        let predecessors = Predecessors {
            sender: Address::repeat_byte(0x11),
            operations: vec![
                (0, Some(B256::repeat_byte(0xaa))),
                (1, Some(B256::repeat_byte(0xbb))),
                (2, Some(B256::repeat_byte(0xcc))),
            ],
        };
        let not_found = Option::<serde_json::Value>::None;

        // Each check reads the pending nonce and looks up only the operation at that nonce.
        asserter.push_success(&U64::from(1));
        asserter.push_failure_msg("lookup unavailable");
        assert_eq!(missing_predecessor(provider.root(), &predecessors).await, None);

        // The gap is reported only if the pending nonce is unchanged after the lookup.
        for recheck in [Some(2), Some(0), None, Some(1)] {
            asserter.push_success(&U64::from(1));
            asserter.push_success(&not_found);
            match recheck {
                Some(nonce) => asserter.push_success(&U64::from(nonce)),
                None => asserter.push_failure_msg("nonce unavailable"),
            }
            let expected = (recheck == Some(1)).then_some(1);
            assert_eq!(missing_predecessor(provider.root(), &predecessors).await, expected);
        }
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn blocked_nonce_requires_a_visible_pending_successor() {
        let (api, handle) = anvil::spawn(anvil::NodeConfig::test().with_no_mining(true)).await;
        let provider = ProviderBuilder::new()
            .connect_http(handle.http_endpoint().parse().unwrap())
            .root()
            .clone();
        let sender = handle.dev_accounts().next().unwrap();
        let successor = send(&provider, sender, 1).await;
        let predecessors =
            Predecessors { sender, operations: vec![(0, Some(B256::repeat_byte(0xab)))] };
        assert_eq!(blocked_nonce(&provider, successor, &predecessors).await, Some(0));
        api.anvil_drop_transaction(successor).await.unwrap();
        assert_eq!(blocked_nonce(&provider, successor, &predecessors).await, None);
        // A fully dropped saved batch must complete reconciliation so resume can replay it.
        let predecessor = predecessors.operations[0].1.unwrap();
        let mut sequence = ScriptSequence::<Ethereum> {
            pending: vec![predecessor, successor],
            ..Default::default()
        };
        ScriptProgress::default()
            .wait_for_pending(
                0,
                &mut sequence,
                &provider,
                1,
                1,
                (
                    &[predecessor, successor],
                    &[predecessor, successor],
                    &[(sender, 0, Some(predecessor)), (sender, 1, Some(successor))],
                ),
            )
            .await
            .unwrap();
        assert!(sequence.pending.is_empty());
        send(&provider, sender, 0).await;
        let successor = send(&provider, sender, 1).await;
        api.mine_one().await.unwrap();
        assert_eq!(blocked_nonce(&provider, successor, &predecessors).await, None);
    }

    #[test]
    fn predecessors_require_the_same_sender_and_lower_nonce() {
        let sender = Address::repeat_byte(1);
        let other = Address::repeat_byte(2);
        let hash = B256::repeat_byte(3);
        let operations =
            [(sender, 0, None), (other, 0, None), (sender, 2, Some(hash)), (sender, 3, None)];
        let predecessors = predecessors(&operations, hash);
        assert_eq!(predecessors.sender, sender);
        assert_eq!(predecessors.operations, vec![(0, None)]);
    }
}
