//! wrappers for transactions
use ethers_core::types::{BlockId, TransactionReceipt};
use ethers_providers::Middleware;
use eyre::Result;
use serde::{Deserialize, Serialize};

/// Helper type to carry a transaction along with an optional revert reason
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransactionReceiptWithRevertReason {
    /// The underlying transaction receipt
    #[serde(flatten)]
    pub receipt: TransactionReceipt,

    /// The revert reason string if the transaction status is failed
    #[serde(skip_serializing_if = "Option::is_none", rename = "revertReason")]
    pub revert_reason: Option<String>,
}

impl TransactionReceiptWithRevertReason {
    /// Returns if the status of the transaction is 0 (failure)
    pub fn is_failure(&self) -> Option<bool> {
        self.receipt.status.map(|status| status.as_u64() == 0)
    }

    /// Updates the revert reason field using `eth_call` and returns an Err variant if the revert
    /// reason was not successfully updated
    pub async fn update_revert_reason<M: Middleware>(&mut self, provider: &M) -> Result<()> {
        self.revert_reason = self.fetch_revert_reason(provider).await?;
        Ok(())
    }

    async fn fetch_revert_reason<M: Middleware>(&self, provider: &M) -> Result<Option<String>> {
        if let Some(false) | None = self.is_failure() {
            return Ok(None)
        }

        if let Some(ref transaction) = provider
            .get_transaction(self.receipt.transaction_hash)
            .await
            .map_err(|_| eyre::eyre!("unable to fetch transaction"))?
        {
            if let Some(block_hash) = self.receipt.block_hash {
                match provider.call(&transaction.into(), Some(BlockId::Hash(block_hash))).await {
                    Err(e) => return Ok(extract_revert_reason(e.to_string())),
                    Ok(_) => eyre::bail!("no revert reason as transaction succeeded"),
                }
            }
            eyre::bail!("unable to fetch block_hash")
        }
        Err(eyre::eyre!("transaction does not exist"))
    }
}

impl From<TransactionReceipt> for TransactionReceiptWithRevertReason {
    fn from(receipt: TransactionReceipt) -> Self {
        Self { receipt, revert_reason: None }
    }
}

impl From<TransactionReceiptWithRevertReason> for TransactionReceipt {
    fn from(receipt_with_reason: TransactionReceiptWithRevertReason) -> Self {
        receipt_with_reason.receipt
    }
}

fn extract_revert_reason<S: AsRef<str>>(error_string: S) -> Option<String> {
    let message_substr = "message: execution reverted: ";

    let mut temp = "";

    error_string
        .as_ref()
        .find(message_substr)
        .and_then(|index| {
            let (_, rest) = error_string.as_ref().split_at(index + message_substr.len());
            temp = rest;
            rest.rfind(", ")
        })
        .map(|index| {
            let (reason, _) = temp.split_at(index);
            reason.to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_revert_reason() {
        let error_string_1 = "(code: 3, message: execution reverted: Transaction too old, data: Some(String(\"0x08c379a0\")))";
        let error_string_2 = "(code: 3, message: execution reverted: missing data: amountIn, amountOut, data: Some(String(\"0x08c379a0\")))";
        let error_string_3 =
            "(code: 4, message: invalid signature, data: Some(String(\"0x08c379a0\")))";

        assert_eq!(extract_revert_reason(error_string_1), Some("Transaction too old".to_string()));
        assert_eq!(
            extract_revert_reason(error_string_2),
            Some("missing data: amountIn, amountOut".to_string())
        );
        assert_eq!(extract_revert_reason(error_string_3), None);
    }
}
