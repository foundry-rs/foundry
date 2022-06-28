use super::sequence::ScriptSequence;
use crate::{init_progress, update_progress, utils::print_receipt};
use ethers::prelude::{Http, PendingTransaction, Provider, RetryClient, TxHash};
use futures::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use std::sync::Arc;

/// Gets the receipts of previously pending transactions.
pub async fn wait_for_pending(
    provider: Arc<Provider<RetryClient<Http>>>,
    deployment_sequence: &mut ScriptSequence,
) -> eyre::Result<()> {
    if !deployment_sequence.pending.is_empty() {
        println!("##\nChecking previously pending transactions.");
        wait_for_receipts(deployment_sequence.pending.clone(), deployment_sequence, provider)
            .await?;
    }
    Ok(())
}

/// Waits for a list of receipts. If it fails, it tries to retrieve the transaction hash that can be
/// used on a later run with `--resume`.
pub async fn wait_for_receipts(
    tx_hashes: Vec<TxHash>,
    deployment_sequence: &mut ScriptSequence,
    provider: Arc<Provider<RetryClient<Http>>>,
) -> eyre::Result<()> {
    let mut tasks = futures::stream::iter(
        tx_hashes.iter().map(|tx| PendingTransaction::new(*tx, &provider)).collect::<Vec<_>>(),
    )
    .buffer_unordered(10);

    let mut receipts = vec![];
    let mut errors: Vec<String> = vec![];
    let pb = init_progress!(tx_hashes, "receipts");
    update_progress!(pb, -1);

    for (index, tx_hash) in tx_hashes.into_iter().enumerate() {
        if let Some(receipt) = tasks.next().await {
            match receipt {
                Ok(Some(receipt)) => {
                    if let Some(status) = receipt.status {
                        if status.is_zero() {
                            errors
                                .push(format!("Transaction Failure: {}", receipt.transaction_hash));
                        }
                    }
                    deployment_sequence.remove_pending(receipt.transaction_hash);
                    receipts.push(receipt)
                }
                Ok(None) => {
                    errors.push(format!("Received an empty receipt for {}", tx_hash));
                }
                Err(err) => {
                    errors.push(format!("Failure on receiving a receipt for {}:\n{err}", tx_hash));
                }
            }
            update_progress!(pb, index);
        } else {
            break
        }
    }

    for receipt in receipts {
        print_receipt(&receipt);
        deployment_sequence.add_receipt(receipt);
    }

    if !errors.is_empty() {
        let mut error_msg = format!("{:?}", errors);
        if !deployment_sequence.pending.is_empty() {
            error_msg += "\n\n Add `--resume` to your command to try and continue broadcasting
    the transactions."
        }
        eyre::bail!(error_msg);
    }

    Ok(())
}
