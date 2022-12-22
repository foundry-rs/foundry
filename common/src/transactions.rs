//! wrappers for transactions
use ethers_core::types::TransactionReceipt;
use serde::{Deserialize, Serialize};

/// Helper type to carry a transaction along with an optional revert reason.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransactionReceiptWithRevertReason {
    /// The underlying transaction receipt
    #[serde(flatten)]
    pub receipt: TransactionReceipt,

    /// The revert reason string if the transaction status is failed
    #[serde(skip_serializing_if = "Option::is_none", rename = "revertReason")]
    pub revert_reason: Option<String>,
}

impl From<TransactionReceipt> for TransactionReceiptWithRevertReason {
    fn from(receipt: TransactionReceipt) -> Self {
        Self { receipt, revert_reason: None }
    }
}
