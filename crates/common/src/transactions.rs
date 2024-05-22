//! wrappers for transactions
use alloy_provider::{network::AnyNetwork, Provider};
use alloy_rpc_types::{AnyTransactionReceipt, BlockId, WithOtherFields};
use alloy_transport::Transport;
use eyre::Result;
use serde::{Deserialize, Serialize};

/// Helper type to carry a transaction along with an optional revert reason
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionReceiptWithRevertReason {
    /// The underlying transaction receipt
    #[serde(flatten)]
    pub receipt: AnyTransactionReceipt,

    /// The revert reason string if the transaction status is failed
    #[serde(skip_serializing_if = "Option::is_none", rename = "revertReason")]
    pub revert_reason: Option<String>,
}

impl TransactionReceiptWithRevertReason {
    /// Returns if the status of the transaction is 0 (failure)
    pub fn is_failure(&self) -> bool {
        !self.receipt.inner.inner.inner.receipt.status
    }

    /// Updates the revert reason field using `eth_call` and returns an Err variant if the revert
    /// reason was not successfully updated
    pub async fn update_revert_reason<T: Transport + Clone, P: Provider<T, AnyNetwork>>(
        &mut self,
        provider: &P,
    ) -> Result<()> {
        self.revert_reason = self.fetch_revert_reason(provider).await?;
        Ok(())
    }

    async fn fetch_revert_reason<T: Transport + Clone, P: Provider<T, AnyNetwork>>(
        &self,
        provider: &P,
    ) -> Result<Option<String>> {
        if !self.is_failure() {
            return Ok(None)
        }

        let transaction = provider
            .get_transaction_by_hash(self.receipt.transaction_hash)
            .await
            .map_err(|err| eyre::eyre!("unable to fetch transaction: {err}"))?
            .ok_or_else(|| eyre::eyre!("transaction not found"))?;

        if let Some(block_hash) = self.receipt.block_hash {
            match provider
                .call(&WithOtherFields::new(transaction.inner.into()))
                .block(BlockId::Hash(block_hash.into()))
                .await
            {
                Err(e) => return Ok(extract_revert_reason(e.to_string())),
                Ok(_) => eyre::bail!("no revert reason as transaction succeeded"),
            }
        }
        eyre::bail!("unable to fetch block_hash")
    }
}

impl From<AnyTransactionReceipt> for TransactionReceiptWithRevertReason {
    fn from(receipt: AnyTransactionReceipt) -> Self {
        Self { receipt, revert_reason: None }
    }
}

impl From<TransactionReceiptWithRevertReason> for AnyTransactionReceipt {
    fn from(receipt_with_reason: TransactionReceiptWithRevertReason) -> Self {
        receipt_with_reason.receipt
    }
}

fn extract_revert_reason<S: AsRef<str>>(error_string: S) -> Option<String> {
    let message_substr = "execution reverted: ";
    error_string
        .as_ref()
        .find(message_substr)
        .map(|index| error_string.as_ref().split_at(index + message_substr.len()).1.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_revert_reason() {
        let error_string_1 = "server returned an error response: error code 3: execution reverted: Transaction too old";
        let error_string_2 = "server returned an error response: error code 3: Invalid signature";

        assert_eq!(extract_revert_reason(error_string_1), Some("Transaction too old".to_string()));
        assert_eq!(extract_revert_reason(error_string_2), None);
    }
}
