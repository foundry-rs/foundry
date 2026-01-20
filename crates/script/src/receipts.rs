use alloy_chains::{Chain, NamedChain};
use alloy_network::AnyTransactionReceipt;
use alloy_primitives::{TxHash, U256, utils::format_units};
use alloy_provider::{PendingTransactionBuilder, PendingTransactionError, Provider, WatchTxError};
use eyre::{Result, eyre};
use forge_script_sequence::ScriptSequence;
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
pub fn format_receipt(
    chain: Chain,
    receipt: &AnyTransactionReceipt,
    sequence: Option<&ScriptSequence>,
) -> String {
    let gas_used = receipt.gas_used;
    let gas_price = receipt.effective_gas_price;
    let block_number = receipt.block_number.unwrap_or_default();
    let success = receipt.inner.inner.inner.receipt.status.coerce_status();

    let (contract_name, function) = sequence
        .and_then(|seq| {
            seq.transactions
                .iter()
                .find(|tx| tx.hash == Some(receipt.transaction_hash))
                .map(|tx| (tx.contract_name.clone(), tx.function.clone()))
        })
        .unwrap_or((None, None));

    if shell::is_json() {
        let mut json = serde_json::json!({
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
        });

        if let Some(name) = &contract_name
            && !name.is_empty()
        {
            json["contract_name"] = serde_json::Value::String(name.clone());
        }
        if let Some(func) = &function
            && !func.is_empty()
        {
            json["function"] = serde_json::Value::String(func.clone());
        }

        let _ = sh_println!("{}", json);

        String::new()
    } else {
        let contract_info = match &contract_name {
            Some(name) if !name.is_empty() => format!("\nContract: {name}"),
            _ => String::new(),
        };

        let function_info = match &function {
            Some(func) if !func.is_empty() => format!("\nFunction: {func}"),
            _ => String::new(),
        };

        format!(
            "\n##### {chain}\n{status} Hash: {tx_hash:?}{contract_info}{function_info}{contract_address}\nBlock: {block_number}\n{gas}\n\n",
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::B256;
    use std::collections::VecDeque;

    fn mock_receipt(tx_hash: B256, success: bool) -> AnyTransactionReceipt {
        serde_json::from_value(serde_json::json!({
            "type": "0x02", "status": if success { "0x1" } else { "0x0" },
            "cumulativeGasUsed": "0x5208", "logs": [], "transactionHash": tx_hash,
            "logsBloom": format!("0x{}", "0".repeat(512)),
            "transactionIndex": "0x0", "blockHash": B256::ZERO, "blockNumber": "0x3039",
            "gasUsed": "0x5208", "effectiveGasPrice": "0x4a817c800",
            "from": "0x0000000000000000000000000000000000000000",
            "to": "0x0000000000000000000000000000000000000000", "contractAddress": null
        }))
        .unwrap()
    }

    fn mock_sequence(tx_hash: B256, contract: Option<&str>, func: Option<&str>) -> ScriptSequence {
        let tx = serde_json::from_value(serde_json::json!({
            "hash": tx_hash, "transactionType": "CALL",
            "contractName": contract, "contractAddress": null, "function": func,
            "arguments": null, "additionalContracts": [], "isFixedGasLimit": false,
            "transaction": {
                "type": "0x02", "chainId": "0x1", "nonce": "0x0", "gas": "0x5208",
                "maxFeePerGas": "0x4a817c800", "maxPriorityFeePerGas": "0x3b9aca00",
                "to": "0x0000000000000000000000000000000000000000",
                "value": "0x0", "input": "0x", "accessList": []
            },
        }))
        .unwrap();
        ScriptSequence {
            transactions: VecDeque::from([tx]),
            chain: 1,
            ..Default::default()
        }
    }

    #[test]
    fn format_receipt_displays_contract_and_function() {
        let hash = B256::repeat_byte(0x42);
        let seq = mock_sequence(hash, Some("MyContract"), Some("init(address)"));
        let out = format_receipt(Chain::mainnet(), &mock_receipt(hash, true), Some(&seq));

        assert!(out.contains("Contract: MyContract"));
        assert!(out.contains("Function: init(address)"));
        assert!(out.contains("✅  [Success]"));
    }

    #[test]
    fn format_receipt_without_sequence_omits_metadata() {
        let hash = B256::repeat_byte(0x42);
        let out = format_receipt(Chain::mainnet(), &mock_receipt(hash, true), None);

        assert!(!out.contains("Contract:"));
        assert!(!out.contains("Function:"));
    }

    #[test]
    fn format_receipt_skips_empty_contract_name() {
        let hash = B256::repeat_byte(0x42);
        let seq = mock_sequence(hash, Some(""), Some("transfer(address)"));
        let out = format_receipt(Chain::mainnet(), &mock_receipt(hash, true), Some(&seq));

        assert!(!out.contains("Contract:"));
        assert!(out.contains("Function: transfer(address)"));
    }

    #[test]
    fn format_receipt_handles_missing_tx_in_sequence() {
        let seq = mock_sequence(B256::repeat_byte(0x99), Some("Other"), Some("other()"));
        let out =
            format_receipt(Chain::mainnet(), &mock_receipt(B256::repeat_byte(0x42), true), Some(&seq));

        assert!(!out.contains("Contract:"));
        assert!(!out.contains("Function:"));
    }

    #[test]
    fn format_receipt_shows_contract_on_failure() {
        let hash = B256::repeat_byte(0x42);
        let seq = mock_sequence(hash, Some("FailContract"), Some("fail()"));
        let out = format_receipt(Chain::mainnet(), &mock_receipt(hash, false), Some(&seq));

        assert!(out.contains("❌  [Failed]"));
        assert!(out.contains("Contract: FailContract"));
    }
}
