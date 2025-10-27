use alloy_primitives::{Address, ChainId, TxHash};
use alloy_rpc_types::TransactionRequest;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Response format for API endpoints.
/// - `Ok(T)` serializes as: {"status":"ok","data": ...}
/// - `Ok(())` serializes as: {"status":"ok"}  (no data key)
/// - `Error { message }` as: {"status":"error","message":"..."}
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "status", content = "data", rename_all = "lowercase")]
pub(crate) enum BrowserApiResponse<T = ()> {
    Ok(T),
    Error { message: String },
}

impl BrowserApiResponse<()> {
    /// Create a successful response with no data.
    pub fn ok() -> Self {
        Self::Ok(())
    }
}

impl<T> BrowserApiResponse<T> {
    /// Create a successful response with the given data.
    pub fn with_data(data: T) -> Self {
        Self::Ok(data)
    }

    /// Create an error response with the given message.
    pub fn error(msg: impl Into<String>) -> Self {
        Self::Error { message: msg.into() }
    }
}

/// Represents a transaction request sent to the browser wallet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserTransaction {
    /// The unique identifier for the transaction.
    pub id: Uuid,
    /// The transaction request details.
    #[serde(flatten)]
    pub request: TransactionRequest,
}

/// Represents a transaction response sent from the browser wallet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TransactionResponse {
    /// The unique identifier for the transaction, must match the request ID sent earlier.
    pub id: Uuid,
    /// The transaction hash if the transaction was successful.
    pub hash: Option<TxHash>,
    /// The error message if the transaction failed.
    pub error: Option<String>,
}

/// Represents an account update sent from the browser wallet.
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection(pub Address, pub ChainId);
