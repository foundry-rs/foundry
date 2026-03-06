//! Wrappers for transactions.

use alloy_consensus::{Transaction, TxEnvelope, transaction::SignerRecoverable};
use alloy_eips::eip7702::SignedAuthorization;
use alloy_network::{AnyTransactionReceipt, Network, TransactionResponse};
use alloy_primitives::{Address, Bytes, TxKind, U256};
use alloy_provider::{
    Provider,
    network::{AnyNetwork, ReceiptResponse, TransactionBuilder},
};
use alloy_rpc_types::{BlockId, TransactionRequest};
use alloy_serde::WithOtherFields;
use eyre::Result;
use foundry_common_fmt::{UIfmt, UIfmtReceiptExt, get_pretty_receipt_attr};
use serde::{Deserialize, Serialize};

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

impl UIfmt for TransactionMaybeSigned {
    fn pretty(&self) -> String {
        match self {
            Self::Signed { tx, .. } => tx.pretty(),
            Self::Unsigned(tx) => format!(
                "
accessList           {}
chainId              {}
gasLimit             {}
gasPrice             {}
input                {}
maxFeePerBlobGas     {}
maxFeePerGas         {}
maxPriorityFeePerGas {}
nonce                {}
to                   {}
type                 {}
value                {}",
                tx.access_list
                    .as_ref()
                    .map(|a| a.iter().collect::<Vec<_>>())
                    .unwrap_or_default()
                    .pretty(),
                tx.chain_id.pretty(),
                tx.gas_limit().unwrap_or_default(),
                tx.gas_price.pretty(),
                tx.input.input.pretty(),
                tx.max_fee_per_blob_gas.pretty(),
                tx.max_fee_per_gas.pretty(),
                tx.max_priority_fee_per_gas.pretty(),
                tx.nonce.pretty(),
                tx.to.as_ref().map(|a| a.to()).unwrap_or_default().pretty(),
                tx.transaction_type.unwrap_or_default(),
                tx.value.pretty(),
            ),
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
    ) -> core::result::Result<Self, alloy_consensus::crypto::RecoveryError> {
        let from = tx.recover_signer()?;
        Ok(Self::Signed { tx, from })
    }

    pub fn is_unsigned(&self) -> bool {
        matches!(self, Self::Unsigned(_))
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

    pub fn input(&self) -> Option<&Bytes> {
        match self {
            Self::Signed { tx, .. } => Some(tx.input()),
            Self::Unsigned(tx) => tx.input.input(),
        }
    }

    pub fn to(&self) -> Option<TxKind> {
        match self {
            Self::Signed { tx, .. } => Some(tx.kind()),
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
            Self::Signed { tx, .. } => Some(tx.gas_limit() as u128),
            Self::Unsigned(tx) => tx.gas_limit().map(|g| g as u128),
        }
    }

    pub fn nonce(&self) -> Option<u64> {
        match self {
            Self::Signed { tx, .. } => Some(tx.nonce()),
            Self::Unsigned(tx) => tx.nonce,
        }
    }

    pub fn authorization_list(&self) -> Option<Vec<SignedAuthorization>> {
        match self {
            Self::Signed { tx, .. } => tx.authorization_list().map(|auths| auths.to_vec()),
            Self::Unsigned(tx) => tx.authorization_list.as_deref().map(|auths| auths.to_vec()),
        }
        .filter(|auths| !auths.is_empty())
    }
}

impl From<TransactionRequest> for TransactionMaybeSigned {
    fn from(tx: TransactionRequest) -> Self {
        Self::new(WithOtherFields::new(tx))
    }
}

impl TryFrom<TxEnvelope> for TransactionMaybeSigned {
    type Error = alloy_consensus::crypto::RecoveryError;

    fn try_from(tx: TxEnvelope) -> core::result::Result<Self, Self::Error> {
        Self::new_signed(tx)
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
