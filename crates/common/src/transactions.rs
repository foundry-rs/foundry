//! Wrappers for transactions.

use alloy_consensus::{Transaction, TxEnvelope};
use alloy_primitives::{Address, TxKind, U256};
use alloy_provider::{network::AnyNetwork, Provider};
use alloy_rpc_types::{AnyTransactionReceipt, BlockId, TransactionRequest};
use alloy_serde::WithOtherFields;
use alloy_transport::Transport;
use eyre::Result;
use foundry_common_fmt::UIfmt;
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
        !self.receipt.inner.inner.inner.receipt.status.coerce_status()
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

impl UIfmt for TransactionReceiptWithRevertReason {
    fn pretty(&self) -> String {
        if let Some(revert_reason) = &self.revert_reason {
            format!(
                "{}
revertReason            {}",
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

/// Returns the `UiFmt::pretty()` formatted attribute of the transaction receipt
pub fn get_pretty_tx_receipt_attr(
    receipt: &TransactionReceiptWithRevertReason,
    attr: &str,
) -> Option<String> {
    match attr {
        "blockHash" | "block_hash" => Some(receipt.receipt.block_hash.pretty()),
        "blockNumber" | "block_number" => Some(receipt.receipt.block_number.pretty()),
        "contractAddress" | "contract_address" => Some(receipt.receipt.contract_address.pretty()),
        "cumulativeGasUsed" | "cumulative_gas_used" => {
            Some(receipt.receipt.inner.inner.inner.receipt.cumulative_gas_used.pretty())
        }
        "effectiveGasPrice" | "effective_gas_price" => {
            Some(receipt.receipt.effective_gas_price.to_string())
        }
        "gasUsed" | "gas_used" => Some(receipt.receipt.gas_used.to_string()),
        "logs" => Some(receipt.receipt.inner.inner.inner.receipt.logs.as_slice().pretty()),
        "logsBloom" | "logs_bloom" => Some(receipt.receipt.inner.inner.inner.logs_bloom.pretty()),
        "root" | "stateRoot" | "state_root " => Some(receipt.receipt.state_root.pretty()),
        "status" | "statusCode" | "status_code" => {
            Some(receipt.receipt.inner.inner.inner.receipt.status.pretty())
        }
        "transactionHash" | "transaction_hash" => Some(receipt.receipt.transaction_hash.pretty()),
        "transactionIndex" | "transaction_index" => {
            Some(receipt.receipt.transaction_index.pretty())
        }
        "type" | "transaction_type" => Some(receipt.receipt.inner.inner.r#type.to_string()),
        "revertReason" | "revert_reason" => Some(receipt.revert_reason.pretty()),
        _ => None,
    }
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

/// Used for broadcasting transactions
/// A transaction can either be a [`TransactionRequest`] waiting to be signed
/// or a [`TxEnvelope`], already signed
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TransactionMaybeSigned {
    Signed {
        #[serde(flatten)]
        tx: TxEnvelope,
        from: Address,
    },
    Unsigned(WithOtherFields<TransactionRequest>),
}

impl TransactionMaybeSigned {
    /// Creates a new (unsigned) transaction for broadcast
    pub fn new(tx: WithOtherFields<TransactionRequest>) -> Self {
        Self::Unsigned(tx)
    }

    /// Creates a new signed transaction for broadcast.
    pub fn new_signed(
        tx: TxEnvelope,
    ) -> core::result::Result<Self, alloy_primitives::SignatureError> {
        let from = tx.recover_signer()?;
        Ok(Self::Signed { tx, from })
    }

    pub fn as_unsigned_mut(&mut self) -> Option<&mut WithOtherFields<TransactionRequest>> {
        match self {
            Self::Unsigned(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn from(&self) -> Option<Address> {
        match self {
            Self::Signed { from, .. } => Some(*from),
            Self::Unsigned(tx) => tx.from,
        }
    }

    pub fn input(&self) -> Option<&[u8]> {
        match self {
            Self::Signed { tx, .. } => Some(tx.input()),
            Self::Unsigned(tx) => tx.input.input().map(|i| i.as_ref()),
        }
    }

    pub fn to(&self) -> Option<TxKind> {
        match self {
            Self::Signed { tx, .. } => Some(tx.to()),
            Self::Unsigned(tx) => tx.to,
        }
    }

    pub fn value(&self) -> Option<U256> {
        match self {
            Self::Signed { tx, .. } => Some(tx.value()),
            Self::Unsigned(tx) => tx.value,
        }
    }

    pub fn gas(&self) -> Option<u128> {
        match self {
            Self::Signed { tx, .. } => Some(tx.gas_limit()),
            Self::Unsigned(tx) => tx.gas,
        }
    }

    pub fn nonce(&self) -> Option<u64> {
        match self {
            Self::Signed { tx, .. } => Some(tx.nonce()),
            Self::Unsigned(tx) => tx.nonce,
        }
    }
}

impl From<TransactionRequest> for TransactionMaybeSigned {
    fn from(tx: TransactionRequest) -> Self {
        Self::new(WithOtherFields::new(tx))
    }
}

impl TryFrom<TxEnvelope> for TransactionMaybeSigned {
    type Error = alloy_primitives::SignatureError;

    fn try_from(tx: TxEnvelope) -> core::result::Result<Self, Self::Error> {
        Self::new_signed(tx)
    }
}
