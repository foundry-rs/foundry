use super::sequence::ScriptSequence;
use alloy_chains::Chain;
use alloy_primitives::{utils::format_units, TxHash, U256};
use alloy_provider::{PendingTransactionBuilder, Provider};
use alloy_rpc_types::AnyTransactionReceipt;
use eyre::Result;
use foundry_cli::{init_progress, update_progress};
use foundry_common::provider::alloy::RetryProvider;
use futures::StreamExt;
use std::sync::Arc;

/// Convenience enum for internal signalling of transaction status
enum TxStatus {
    Dropped,
    Success(AnyTransactionReceipt),
    Revert(AnyTransactionReceipt),
}

impl From<AnyTransactionReceipt> for TxStatus {
    fn from(receipt: AnyTransactionReceipt) -> Self {
        if !receipt.inner.inner.inner.receipt.status {
            TxStatus::Revert(receipt)
        } else {
            TxStatus::Success(receipt)
        }
    }
}

/// Gets the receipts of previously pending transactions, or removes them from
/// the deploy sequence's pending vector
pub async fn wait_for_pending(
    provider: Arc<RetryProvider>,
    deployment_sequence: &mut ScriptSequence,
) -> Result<()> {
    if deployment_sequence.pending.is_empty() {
        return Ok(());
    }
    println!("##\nChecking previously pending transactions.");
    clear_pendings(provider, deployment_sequence, None).await
}

/// Traverses a set of pendings and either finds receipts, or clears them from
/// the deployment sequence.
///
/// If no `tx_hashes` are provided, then `deployment_sequence.pending` will be
/// used. For each `tx_hash`, we check if it has confirmed. If it has
/// confirmed, we push the receipt (if successful) or push an error (if
/// revert). If the transaction has not confirmed, but can be found in the
/// node's mempool, we wait for its receipt to be available. If the transaction
/// has not confirmed, and cannot be found in the mempool, we remove it from
/// the `deploy_sequence.pending` vector so that it will be rebroadcast in
/// later steps.
pub async fn clear_pendings(
    provider: Arc<RetryProvider>,
    deployment_sequence: &mut ScriptSequence,
    tx_hashes: Option<Vec<TxHash>>,
) -> Result<()> {
    let to_query = tx_hashes.unwrap_or_else(|| deployment_sequence.pending.clone());

    let count = deployment_sequence.pending.len();

    trace!("Checking status of {count} pending transactions");

    let futs = to_query.iter().copied().map(|tx| check_tx_status(&provider, tx));
    let mut tasks = futures::stream::iter(futs).buffer_unordered(10);

    let mut errors: Vec<String> = vec![];
    let mut receipts = Vec::<AnyTransactionReceipt>::with_capacity(count);

    // set up progress bar
    let mut pos = 0;
    let pb = init_progress!(deployment_sequence.pending, "receipts");
    pb.set_position(pos);

    while let Some((tx_hash, result)) = tasks.next().await {
        match result {
            Err(err) => {
                errors.push(format!("Failure on receiving a receipt for {tx_hash:?}:\n{err}"))
            }
            Ok(TxStatus::Dropped) => {
                // We want to remove it from pending so it will be re-broadcast.
                deployment_sequence.remove_pending(tx_hash);
                errors.push(format!("Transaction dropped from the mempool: {tx_hash:?}"));
            }
            Ok(TxStatus::Success(receipt)) => {
                trace!(tx_hash=?tx_hash, "received tx receipt");
                deployment_sequence.remove_pending(receipt.transaction_hash);
                receipts.push(receipt);
            }
            Ok(TxStatus::Revert(receipt)) => {
                // consider:
                // if this is not removed from pending, then the script becomes
                // un-resumable. Is this desirable on reverts?
                warn!(tx_hash=?tx_hash, "Transaction Failure");
                deployment_sequence.remove_pending(receipt.transaction_hash);
                errors.push(format!("Transaction Failure: {:?}", receipt.transaction_hash));
            }
        }
        // update the progress bar
        update_progress!(pb, pos);
        pos += 1;
    }

    // sort receipts by blocks asc and index
    receipts.sort_by_key(|r| (r.block_number, r.transaction_index));

    // print all receipts
    for receipt in receipts {
        print_receipt(deployment_sequence.chain.into(), &receipt);
        deployment_sequence.add_receipt(receipt);
    }

    // print any errors
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

/// Checks the status of a txhash by first polling for a receipt, then for
/// mempool inclusion. Returns the tx hash, and a status
async fn check_tx_status(
    provider: &RetryProvider,
    hash: TxHash,
) -> (TxHash, Result<TxStatus, eyre::Report>) {
    // We use the inner future so that we can use ? operator in the future, but
    // still neatly return the tuple
    let result = async move {
        // First check if there's a receipt
        let receipt_opt = provider.get_transaction_receipt(hash).await?;
        if let Some(receipt) = receipt_opt {
            return Ok(receipt.into());
        }

        // If the tx is present in the mempool, run the pending tx future, and
        // assume the next drop is really really real
        Ok(PendingTransactionBuilder::new(provider, hash)
            .get_receipt()
            .await
            .map_or(TxStatus::Dropped, |r| r.into()))
    }
    .await;

    (hash, result)
}

/// Prints parts of the receipt to stdout
pub fn print_receipt(chain: Chain, receipt: &AnyTransactionReceipt) {
    let gas_used = receipt.gas_used.unwrap_or_default();
    let gas_price = receipt.effective_gas_price;
    foundry_common::shell::println(format!(
        "\n##### {chain}\n{status}Hash: {tx_hash:?}{caddr}\nBlock: {bn}\n{gas}\n",
        status = if !receipt.inner.inner.inner.receipt.status {
            "❌  [Failed]"
        } else {
            "✅  [Success]"
        },
        tx_hash = receipt.transaction_hash,
        caddr = if let Some(addr) = &receipt.contract_address {
            format!("\nContract Address: {}", addr.to_checksum(None))
        } else {
            String::new()
        },
        bn = receipt.block_number.unwrap_or_default(),
        gas = if gas_price == 0 {
            format!("Gas Used: {gas_used}")
        } else {
            let paid = format_units(gas_used.saturating_mul(gas_price), 18)
                .unwrap_or_else(|_| "N/A".into());
            let gas_price = format_units(U256::from(gas_price), 9).unwrap_or_else(|_| "N/A".into());
            format!(
                "Paid: {} ETH ({gas_used} gas * {} gwei)",
                paid.trim_end_matches('0'),
                gas_price.trim_end_matches('0').trim_end_matches('.')
            )
        },
    ))
    .expect("could not print receipt");
}
