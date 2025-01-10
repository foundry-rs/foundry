use alloy_chains::Chain;
use alloy_network::AnyTransactionReceipt;
use alloy_primitives::{utils::format_units, TxHash, U256};
use alloy_provider::{PendingTransactionBuilder, PendingTransactionError, Provider, WatchTxError};
use eyre::{eyre, Result};
use foundry_common::{provider::RetryProvider, retry, retry::RetryError, shell};
use std::time::Duration;

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
                Ok(receipt) => Ok(receipt.into()),
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
                format!(
                    "Paid: {} ETH ({gas_used} gas * {} gwei)",
                    paid.trim_end_matches('0'),
                    gas_price.trim_end_matches('0').trim_end_matches('.')
                )
            },
        )
    }
}
