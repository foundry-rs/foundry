use alloy_chains::{Chain, NamedChain};
use alloy_network::AnyTransactionReceipt;
use alloy_primitives::{TxHash, U256, utils::format_units};
use alloy_provider::{PendingTransactionBuilder, PendingTransactionError, Provider, WatchTxError};
use eyre::{Result, eyre};
use foundry_common::{provider::RetryProvider, retry, retry::RetryError, shell};
use std::time::Duration;

/// Marker error type for pending receipts
#[derive(Debug, thiserror::Error)]
#[error(
    "Received a pending receipt for {tx_hash}, but transaction is still known to the node, retrying"
)]
pub struct PendingReceiptError {
    pub tx_hash: TxHash,
}

/// Convenience enum for internal signalling of transaction status
pub enum TxStatus {
    Dropped,
    Success(AnyTransactionReceipt),
    Revert(AnyTransactionReceipt),
}

impl From<AnyTransactionReceipt> for TxStatus {
    fn from(receipt: AnyTransactionReceipt) -> Self {
        if !receipt.inner.inner.inner.receipt.status.coerce_status() {
            Self::Revert(receipt)
        } else {
            Self::Success(receipt)
        }
    }
}

/// Checks the status of a txhash by first polling for a receipt, then for
/// mempool inclusion. Returns the tx hash, and a status
pub async fn check_tx_status(
    provider: &RetryProvider,
    hash: TxHash,
    timeout: u64,
) -> (TxHash, Result<TxStatus, eyre::Report>) {
    let result = retry::Retry::new_no_delay(3)
        .run_async_until_break(|| async {
            match PendingTransactionBuilder::new(provider.clone(), hash)
                .with_timeout(Some(Duration::from_secs(timeout)))
                .get_receipt()
                .await
            {
                Ok(receipt) => {
                    // Check if the receipt is pending (missing block information)
                    let is_pending = receipt.block_number.is_none()
                        || receipt.block_hash.is_none()
                        || receipt.transaction_index.is_none();

                    if !is_pending {
                        return Ok(receipt.into());
                    }

                    // Receipt is pending, try to sleep and retry a few times
                    match provider.get_transaction_by_hash(hash).await {
                        Ok(_) => {
                            // Sleep for a short time to allow the transaction to be mined
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            // Transaction is still known to the node, retry
                            Err(RetryError::Retry(PendingReceiptError { tx_hash: hash }.into()))
                        }
                        Err(_) => {
                            // Transaction is not known to the node, mark it as dropped
                            Ok(TxStatus::Dropped)
                        }
                    }
                }
                Err(e) => match provider.get_transaction_by_hash(hash).await {
                    Ok(_) => match e {
                        PendingTransactionError::TxWatcher(WatchTxError::Timeout) => {
                            Err(RetryError::Continue(eyre!(
                                "tx is still known to the node, waiting for receipt"
                            )))
                        }
                        _ => Err(RetryError::Retry(e.into())),
                    },
                    Err(_) => Ok(TxStatus::Dropped),
                },
            }
        })
        .await;

    (hash, result)
}

/// Prints parts of the receipt to stdout
pub fn format_receipt(chain: Chain, receipt: &AnyTransactionReceipt) -> String {
    let gas_used = receipt.gas_used;
    let gas_price = receipt.effective_gas_price;
    let block_number = receipt.block_number.unwrap_or_default();
    let success = receipt.inner.inner.inner.receipt.status.coerce_status();

    if shell::is_json() {
        let _ = sh_println!(
            "{}",
            serde_json::json!({
                "chain": chain,
                "status": if success {
                    "success"
                } else {
                    "failed"
                },
                "tx_hash": receipt.transaction_hash,
                "contract_address": receipt.contract_address.map(|addr| addr.to_string()),
                "block_number": block_number,
                "gas_used": gas_used,
                "gas_price": gas_price,
            })
        );

        String::new()
    } else {
        format!(
            "\n##### {chain}\n{status} Hash: {tx_hash:?}{contract_address}\nBlock: {block_number}\n{gas}\n\n",
            status = if success { "✅  [Success]" } else { "❌  [Failed]" },
            tx_hash = receipt.transaction_hash,
            contract_address = if let Some(addr) = &receipt.contract_address {
                format!("\nContract Address: {}", addr.to_checksum(None))
            } else {
                String::new()
            },
            gas = if gas_price == 0 {
                format!("Gas Used: {gas_used}")
            } else {
                let paid = format_units((gas_used as u128).saturating_mul(gas_price), 18)
                    .unwrap_or_else(|_| "N/A".into());
                let gas_price =
                    format_units(U256::from(gas_price), 9).unwrap_or_else(|_| "N/A".into());
                let token_symbol = NamedChain::try_from(chain)
                    .unwrap_or_default()
                    .native_currency_symbol()
                    .unwrap_or("ETH");
                format!(
                    "Paid: {} {} ({gas_used} gas * {} gwei)",
                    paid.trim_end_matches('0'),
                    token_symbol,
                    gas_price.trim_end_matches('0').trim_end_matches('.')
                )
            },
        )
    }
}
