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

/// Contains information about the active wallet connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletConnection {
    /// The address of the connected wallet.
    pub address: Address,
    /// The chain ID of the connected wallet.
    pub chain_id: ChainId,
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
/// If `address` is `None`, it indicates that the wallet has disconnected.
/// If `address` is different from the previous one, it indicates a switch to a new account.
/// If `chain_id` is provided, it indicates a change in the connected chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AccountUpdate {
    /// The address of the account.
    pub address: Option<Address>,
    /// The chain ID of the account.
    pub chain_id: Option<ChainId>,
}
