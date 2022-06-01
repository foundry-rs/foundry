use crate::{cmd::ScriptSequence, utils::print_receipt};
use ethers::prelude::{Http, Middleware, Provider, TransactionReceipt, H256, U256};
use futures::future::join_all;

use super::broadcast::BroadcastError;

/// Gets the receipts of previously pending transactions.
pub async fn wait_for_pending(
    provider: &Provider<Http>,
    deployment_sequence: &mut ScriptSequence,
) -> eyre::Result<()> {
    if !deployment_sequence.pending.is_empty() {
        println!("##\nChecking previously pending transactions.");
        let future_receipts = deployment_sequence
            .pending
            .iter()
            .map(|tx_hash| pending_receipt(provider, *tx_hash))
            .collect();
        wait_for_receipts(future_receipts, deployment_sequence).await?;
    }
    Ok(())
}

/// Waits for a pending receipt, and gets its nonce to return (receipt, nonce).
async fn pending_receipt(
    provider: &Provider<Http>,
    tx_hash: H256,
) -> Result<(TransactionReceipt, U256), BroadcastError> {
    let pending_err =
        || BroadcastError::Simple(format!("Failed to get pending transaction {tx_hash:?}."));

    let receipt = provider
        .get_transaction_receipt(tx_hash)
        .await
        .map_err(|_| pending_err())?
        .ok_or_else(pending_err)?;

    let tx = provider
        .get_transaction(tx_hash)
        .await
        .map_err(|_| pending_err())?
        .ok_or_else(pending_err)?;

    Ok((receipt, tx.nonce))
}

/// Waits for a list of receipts. If it fails, it tries to retrieve the transaction hash that can be
/// used on a later run with `--resume`.
pub async fn wait_for_receipts(
    tasks: Vec<impl futures::Future<Output = Result<(TransactionReceipt, U256), BroadcastError>>>,
    deployment_sequence: &mut ScriptSequence,
) -> eyre::Result<()> {
    let res = join_all(tasks).await;

    let mut receipts = vec![];
    let mut errors: Vec<String> = vec![];

    for receipt in res {
        match receipt {
            Ok(ret) => {
                if let Some(status) = ret.0.status {
                    if status.is_zero() {
                        errors.push(format!("Transaction Failure: {}", ret.0.transaction_hash));
                    }
                }
                deployment_sequence.remove_pending(ret.0.transaction_hash);
                let _ = print_receipt(&ret.0, ret.1);
                receipts.push(ret)
            }
            Err(e) => {
                if let BroadcastError::ErrorWithTxHash(_, tx_hash) = e {
                    deployment_sequence.add_pending(tx_hash);
                }
                errors.push(format!("{e}"));
            }
        };
    }

    for (receipt, nonce) in receipts {
        print_receipt(&receipt, nonce)?;
        deployment_sequence.add_receipt(receipt);
    }

    if !errors.is_empty() {
        let mut error_msg = format!("{:?}", errors);
        if !deployment_sequence.pending.is_empty() {
            error_msg += "\n\n Add `--resume` to your command to try and continue broadcasting the transactions."
        }
        eyre::bail!(error_msg);
    }

    Ok(())
}
