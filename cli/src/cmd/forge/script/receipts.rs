use std::collections::BTreeMap;

use crate::{
    cmd::ScriptSequence,
    utils::{get_http_provider, print_receipt},
};
use ethers::{
    abi::Address,
    prelude::{Bytes, Middleware, TransactionReceipt, TxHash, U256},
    types::transaction::eip2718::TypedTransaction,
};
use futures::future::join_all;

use super::broadcast::BroadcastError;

// Get all transactions in the `tx_hashes` list.
pub async fn get_pending_txes(
    tx_hashes: &Vec<TxHash>,
    fork_url: &str,
) -> BTreeMap<(Address, U256), (Bytes, TxHash)> {
    let provider = get_http_provider(fork_url);
    let mut pending_txes = BTreeMap::new();
    for pending in tx_hashes {
        if let Ok(Some(tx)) = provider.get_transaction(*pending).await {
            pending_txes.insert((tx.from, tx.nonce), (tx.input, *pending));
        }
    }
    pending_txes
}

/// Given a tx, it checks if there is already a receipt for it. Compares `from`, `nonce` and `data`.
pub async fn maybe_has_receipt(
    tx: &TypedTransaction,
    pending_txes: &BTreeMap<(Address, U256), (Bytes, TxHash)>,
    fork_url: &str,
) -> Option<TransactionReceipt> {
    let mut receipt = None;
    if let Some((data, tx_hash)) = pending_txes.get(&(*tx.from().unwrap(), *tx.nonce().unwrap())) {
        if tx.data().unwrap().eq(data) {
            let provider = get_http_provider(fork_url);
            if let Ok(ret) = provider.get_transaction_receipt(*tx_hash).await {
                receipt = ret
            }
        }
    }
    receipt
}

/// Waits for a list of receipts. If it fails, it tries to retrieve the transaction hash that can be
/// used on a later run with `--resume`.
pub async fn wait_for_receipts(
    tasks: Vec<impl futures::Future<Output = Result<(TransactionReceipt, U256), BroadcastError>>>,
    deployment_sequence: &mut ScriptSequence,
) -> eyre::Result<()> {
    let res = join_all(tasks).await;

    let mut receipts = vec![];
    let mut errors = vec![];

    for receipt in res {
        match receipt {
            Ok(ret) => {
                if let Some(status) = ret.0.status {
                    if status.is_zero() {
                        errors.push(format!("Transaction Failure: {}", ret.0.transaction_hash));
                    }
                }
                receipts.push(ret)
            }
            Err(e) => {
                let err = match e {
                    BroadcastError::Simple(err) => err,
                    BroadcastError::ErrorWithTxHash(err, tx_hash) => {
                        deployment_sequence.add_pending(tx_hash);
                        format!("\nFailed to wait for transaction:{tx_hash}:\n{err}")
                    }
                };
                errors.push(err)
            }
        };
    }

    // Receipts may have arrived out of order
    receipts.sort_by(|a, b| a.1.cmp(&b.1));
    for (receipt, nonce) in receipts {
        print_receipt(&receipt, nonce)?;
        deployment_sequence.add_receipt(receipt);
    }

    if !errors.is_empty() {
        eyre::bail!(format!("{:?}", errors));
    }

    Ok(())
}
