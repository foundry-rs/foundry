use alloy_network::{AnyNetwork, AnyTransactionReceipt, Network, TransactionResponse};
use alloy_primitives::Address;
use alloy_provider::{
    Provider,
    network::{ReceiptResponse, TransactionBuilder},
};
use alloy_rpc_types::{BlockId, TransactionReceipt};
use eyre::Result;
use foundry_common_fmt::{UIfmt, UIfmtReceiptExt, get_pretty_receipt_attr};
use serde::{Deserialize, Serialize};
use tempo_alloy::rpc::TempoTransactionReceipt;

/// Helper trait providing `contract_address` setter for generic `ReceiptResponse`
pub trait FoundryReceiptResponse {
    /// Sets address of the created contract, or `None` if the transaction was not a deployment.
    fn set_contract_address(&mut self, contract_address: Address);
}

impl FoundryReceiptResponse for TransactionReceipt {
    fn set_contract_address(&mut self, contract_address: Address) {
        self.contract_address = Some(contract_address);
    }
}

impl FoundryReceiptResponse for TempoTransactionReceipt {
    fn set_contract_address(&mut self, contract_address: Address) {
        self.contract_address = Some(contract_address);
    }
}

/// Helper type to carry a transaction along with an optional revert reason
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionReceiptWithRevertReason<N: Network> {
    /// The underlying transaction receipt
    #[serde(flatten)]
    pub receipt: N::ReceiptResponse,

    /// The revert reason string if the transaction status is failed
    #[serde(skip_serializing_if = "Option::is_none", rename = "revertReason")]
    pub revert_reason: Option<String>,
}

impl<N: Network> TransactionReceiptWithRevertReason<N>
where
    N::TxEnvelope: Clone,
    N::ReceiptResponse: UIfmtReceiptExt,
{
    /// Updates the revert reason field using `eth_call` and returns an Err variant if the revert
    /// reason was not successfully updated
    pub async fn update_revert_reason(&mut self, provider: &dyn Provider<N>) -> Result<()> {
        self.revert_reason = self.fetch_revert_reason(provider).await?;
        Ok(())
    }

    async fn fetch_revert_reason(&self, provider: &dyn Provider<N>) -> Result<Option<String>> {
        // If the transaction succeeded, there is no revert reason to fetch
        if self.receipt.status() {
            return Ok(None);
        }

        let transaction = provider
            .get_transaction_by_hash(self.receipt.transaction_hash())
            .await
            .map_err(|err| eyre::eyre!("unable to fetch transaction: {err}"))?
            .ok_or_else(|| eyre::eyre!("transaction not found"))?;

        if let Some(block_hash) = self.receipt.block_hash() {
            let mut call_request: N::TransactionRequest = transaction.as_ref().clone().into();
            call_request.set_from(transaction.from());
            match provider.call(call_request).block(BlockId::Hash(block_hash.into())).await {
                Err(e) => return Ok(extract_revert_reason(e.to_string())),
                Ok(_) => eyre::bail!("no revert reason as transaction succeeded"),
            }
        }
        eyre::bail!("unable to fetch block_hash")
    }
}

impl From<AnyTransactionReceipt> for TransactionReceiptWithRevertReason<AnyNetwork> {
    fn from(receipt: AnyTransactionReceipt) -> Self {
        Self { receipt, revert_reason: None }
    }
}

impl From<TransactionReceiptWithRevertReason<AnyNetwork>> for AnyTransactionReceipt {
    fn from(receipt_with_reason: TransactionReceiptWithRevertReason<AnyNetwork>) -> Self {
        receipt_with_reason.receipt
    }
}

impl<N: Network> UIfmt for TransactionReceiptWithRevertReason<N>
where
    N::ReceiptResponse: UIfmt,
{
    fn pretty(&self) -> String {
        if let Some(revert_reason) = &self.revert_reason {
            format!(
                "{}
revertReason         {}",
                self.receipt.pretty(),
                revert_reason
            )
        } else {
            self.receipt.pretty()
        }
    }
}

fn extract_revert_reason<S: AsRef<str>>(error_string: S) -> Option<String> {
    let message_substr = "execution reverted: ";
    error_string
        .as_ref()
        .find(message_substr)
        .map(|index| error_string.as_ref().split_at(index + message_substr.len()).1.to_string())
}

/// Returns the `UiFmt::pretty()` formatted attribute of the transaction receipt with revert reason
pub fn get_pretty_receipt_w_reason_attr<N>(
    receipt: &TransactionReceiptWithRevertReason<N>,
    attr: &str,
) -> Option<String>
where
    N: Network,
    N::ReceiptResponse: UIfmtReceiptExt,
{
    // Handle revert reason first, then delegate to the receipt formatting function
    if matches!(attr, "revertReason" | "revert_reason") {
        return Some(receipt.revert_reason.pretty());
    }
    get_pretty_receipt_attr::<N>(&receipt.receipt, attr)
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
