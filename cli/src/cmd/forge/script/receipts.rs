use crate::{
    cmd::forge::script::sequence::ScriptSequence, init_progress, update_progress,
    utils::print_receipt,
};
use ethers::prelude::{PendingTransaction, TxHash};
use foundry_common::RetryProvider;
use futures::StreamExt;
use std::sync::Arc;
use tracing::trace;

/// Gets the receipts of previously pending transactions.
pub async fn wait_for_pending(
    provider: Arc<RetryProvider>,
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
    provider: Arc<RetryProvider>,
) -> eyre::Result<()> {
    trace!("waiting for receipts of {} transactions", tx_hashes.len());
    let mut tasks = futures::stream::iter(
        tx_hashes.iter().map(|tx| PendingTransaction::new(*tx, &provider)).collect::<Vec<_>>(),
    )
    .buffer_unordered(10);

    let mut receipts = Vec::with_capacity(tx_hashes.len());
    let mut errors: Vec<String> = vec![];
    let pb = init_progress!(tx_hashes, "receipts");
    pb.set_position(0);

    for (index, tx_hash) in tx_hashes.into_iter().enumerate() {
        if let Some(receipt) = tasks.next().await {
            match receipt {
                Ok(Some(receipt)) => {
                    if let Some(status) = receipt.status {
                        if status.is_zero() {
                            errors.push(format!(
                                "Transaction Failure: {:?}",
                                receipt.transaction_hash
                            ));
                        }
                    }
                    trace!(?receipt.transaction_hash, "received tx receipt");

                    deployment_sequence.remove_pending(receipt.transaction_hash);
                    receipts.push(receipt)
                }
                Ok(None) => {
                    errors.push(format!(
                        "Transaction unavailable in mempool but not confirmed: {tx_hash:?}. This commonly occurs when connected to public RPCs."
                    ));
                }
                Err(err) => {
                    errors.push(format!("Failure on receiving a receipt for {tx_hash:?}:\n{err}"));
                }
            }
            update_progress!(pb, index);
        } else {
            break
        }
    }

    // sort receipts by blocks asc and index
    receipts.sort_unstable();

    for receipt in receipts {
        print_receipt(deployment_sequence.chain.into(), &receipt);
        deployment_sequence.add_receipt(receipt);
    }

    if !errors.is_empty() {
        let mut error_msg = errors.join("\n");
        if !deployment_sequence.pending.is_empty() {
            error_msg += "\n\n Add `--resume` to your command to try and continue broadcasting
    the transactions."
        }
        eyre::bail!(error_msg);
    }

    Ok(())
}
