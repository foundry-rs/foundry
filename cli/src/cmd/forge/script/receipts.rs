use crate::{cmd::ScriptSequence, utils::print_receipt};
use ethers::prelude::{Http, PendingTransaction, Provider, TxHash};
use futures::StreamExt;

use super::broadcast::BroadcastError;

/// Gets the receipts of previously pending transactions.
pub async fn wait_for_pending(
    provider: &Provider<Http>,
    deployment_sequence: &mut ScriptSequence,
) -> eyre::Result<()> {
    if !deployment_sequence.pending.is_empty() {
        println!("##\nChecking previously pending transactions.");
        wait_for_receipts(
            &deployment_sequence.pending.iter().map(|tx_hash| Ok(*tx_hash)).collect::<Vec<_>>(),
            deployment_sequence,
            provider,
        )
        .await?;
    }
    Ok(())
}

/// Waits for a list of receipts. If it fails, it tries to retrieve the transaction hash that can be
/// used on a later run with `--resume`.
pub async fn wait_for_receipts(
    tx_hashes: &[Result<TxHash, BroadcastError>],
    deployment_sequence: &mut ScriptSequence,
    provider: &Provider<Http>,
) -> eyre::Result<()> {
    let mut tasks = vec![];
    for tx_hash in tx_hashes {
        tasks.push(PendingTransaction::new(tx_hash.clone()?, provider));
    }

    let tasks = futures::stream::iter(tasks).buffered(20);
    let mut receipts = vec![];
    let mut errors: Vec<String> = vec![];

    for (tx_hash, receipt) in tx_hashes.iter().zip(tasks.collect::<Vec<_>>().await) {
        let tx_hash = tx_hash.clone()?;

        match receipt {
            Ok(Some(receipt)) => {
                if let Some(status) = receipt.status {
                    if status.is_zero() {
                        errors.push(format!("Transaction Failure: {}", receipt.transaction_hash));
                    }
                }
                deployment_sequence.remove_pending(receipt.transaction_hash);
                let _ = print_receipt(&receipt);
                receipts.push(receipt)
            }
            Ok(None) | Err(_) => {
                deployment_sequence.add_pending(tx_hash);
                errors.push(format!("Failure on receiving a receipt for {}", tx_hash));
            }
        }
    }

    for receipt in receipts {
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
