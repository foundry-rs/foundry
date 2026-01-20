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

    fn create_mock_receipt(tx_hash: B256, success: bool) -> AnyTransactionReceipt {
        let status = if success { "0x1" } else { "0x0" };
        let receipt_json = serde_json::json!({
            "type": "0x02",
            "status": status,
            "cumulativeGasUsed": "0x5208",
            "logs": [],
            "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "transactionHash": tx_hash,
            "transactionIndex": "0x0",
            "blockHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "blockNumber": "0x3039",
            "gasUsed": "0x5208",
            "effectiveGasPrice": "0x4a817c800",
            "from": "0x0000000000000000000000000000000000000000",
            "to": "0x0000000000000000000000000000000000000000",
            "contractAddress": null
        });
        serde_json::from_value(receipt_json).expect("valid receipt json")
    }

    fn create_mock_sequence_with_tx(
        tx_hash: B256,
        contract_name: Option<String>,
        function: Option<String>,
    ) -> ScriptSequence {
        let tx_json = serde_json::json!({
            "hash": tx_hash,
            "transactionType": "CALL",
            "contractName": contract_name,
            "contractAddress": null,
            "function": function,
            "arguments": null,
            "transaction": {
                "type": "0x02",
                "chainId": "0x1",
                "nonce": "0x0",
                "gas": "0x5208",
                "maxFeePerGas": "0x4a817c800",
                "maxPriorityFeePerGas": "0x3b9aca00",
                "to": "0x0000000000000000000000000000000000000000",
                "value": "0x0",
                "input": "0x",
                "accessList": []
            },
            "additionalContracts": [],
            "isFixedGasLimit": false
        });

        let tx: forge_script_sequence::TransactionWithMetadata =
            serde_json::from_value(tx_json).expect("valid tx json");

        let mut transactions = VecDeque::new();
        transactions.push_back(tx);

        ScriptSequence {
            transactions,
            receipts: vec![],
            libraries: vec![],
            pending: vec![],
            paths: None,
            returns: Default::default(),
            timestamp: 0,
            chain: 1,
            commit: None,
        }
    }

    #[test]
    fn test_format_receipt_with_contract_and_function() {
        let tx_hash = B256::repeat_byte(0x42);
        let receipt = create_mock_receipt(tx_hash, true);
        let sequence = create_mock_sequence_with_tx(
            tx_hash,
            Some("MyContract".to_string()),
            Some("initialize(address,uint256)".to_string()),
        );

        let output = format_receipt(Chain::mainnet(), &receipt, Some(&sequence));

        assert!(output.contains("Contract: MyContract"), "Output should contain contract name");
        assert!(
            output.contains("Function: initialize(address,uint256)"),
            "Output should contain function signature"
        );
        assert!(output.contains("✅  [Success]"), "Output should show success status");
    }

    #[test]
    fn test_format_receipt_without_sequence() {
        let tx_hash = B256::repeat_byte(0x42);
        let receipt = create_mock_receipt(tx_hash, true);

        let output = format_receipt(Chain::mainnet(), &receipt, None);

        assert!(!output.contains("Contract:"), "Output should not contain contract label");
        assert!(!output.contains("Function:"), "Output should not contain function label");
        assert!(output.contains("✅  [Success]"), "Output should show success status");
    }

    #[test]
    fn test_format_receipt_with_empty_contract_name() {
        let tx_hash = B256::repeat_byte(0x42);
        let receipt = create_mock_receipt(tx_hash, true);
        let sequence = create_mock_sequence_with_tx(
            tx_hash,
            Some(String::new()),
            Some("transfer(address,uint256)".to_string()),
        );

        let output = format_receipt(Chain::mainnet(), &receipt, Some(&sequence));

        assert!(!output.contains("Contract:"), "Output should not contain empty contract name");
        assert!(
            output.contains("Function: transfer(address,uint256)"),
            "Output should contain function signature"
        );
    }

    #[test]
    fn test_format_receipt_tx_not_found_in_sequence() {
        let tx_hash = B256::repeat_byte(0x42);
        let different_hash = B256::repeat_byte(0x99);
        let receipt = create_mock_receipt(tx_hash, true);
        let sequence = create_mock_sequence_with_tx(
            different_hash,
            Some("OtherContract".to_string()),
            Some("otherFunction()".to_string()),
        );

        let output = format_receipt(Chain::mainnet(), &receipt, Some(&sequence));

        assert!(
            !output.contains("Contract:"),
            "Output should not contain contract when tx not found"
        );
        assert!(
            !output.contains("Function:"),
            "Output should not contain function when tx not found"
        );
    }

    #[test]
    fn test_format_receipt_failed_transaction() {
        let tx_hash = B256::repeat_byte(0x42);
        let receipt = create_mock_receipt(tx_hash, false);
        let sequence = create_mock_sequence_with_tx(
            tx_hash,
            Some("FailingContract".to_string()),
            Some("failingFunction()".to_string()),
        );

        let output = format_receipt(Chain::mainnet(), &receipt, Some(&sequence));

        assert!(output.contains("❌  [Failed]"), "Output should show failed status");
        assert!(
            output.contains("Contract: FailingContract"),
            "Output should contain contract name even on failure"
        );
    }
}
