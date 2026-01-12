use alloy_dyn_abi::TypedData;
use alloy_primitives::{Address, Bytes, ChainId, TxHash};
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
#[serde(deny_unknown_fields)]
pub struct BrowserTransactionRequest {
    /// The unique identifier for the transaction.
    pub id: Uuid,
    /// The transaction request details.
    pub request: TransactionRequest,
}

/// Represents a transaction response sent from the browser wallet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrowserTransactionResponse {
    /// The unique identifier for the transaction, must match the request ID sent earlier.
    pub id: Uuid,
    /// The transaction hash if the transaction was successful.
    pub hash: Option<TxHash>,
    /// The error message if the transaction failed.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum SignType {
    /// Standard personal sign: `eth_sign` / `personal_sign`
    PersonalSign,
    /// EIP-712 typed data sign: `eth_signTypedData_v4`
    SignTypedDataV4,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignRequest {
    /// The message to be signed.
    pub message: String,
    /// The address that should sign the message.
    pub address: Address,
}

/// Represents a signing request sent to the browser wallet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserSignRequest {
    /// The unique identifier for the signing request.
    pub id: Uuid,
    /// The type of signing operation.
    pub sign_type: SignType,
    /// The sign request details.
    pub request: SignRequest,
}

/// Represents a typed data signing request sent to the browser wallet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserSignTypedDataRequest {
    /// The unique identifier for the signing request.
    pub id: Uuid,
    /// The address that should sign the typed data.
    pub address: Address,
    /// The typed data to be signed.
    pub typed_data: TypedData,
}

/// Represents a signing response sent from the browser wallet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrowserSignResponse {
    /// The unique identifier for the signing request, must match the request ID sent earlier.
    pub id: Uuid,
    /// The signature if the signing was successful.
    pub signature: Option<Bytes>,
    /// The error message if the signing failed.
    pub error: Option<String>,
}

/// Represents an active connection to a browser wallet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    /// The address of the connected wallet.
    pub address: Address,
    /// The chain ID of the connected wallet.
    pub chain_id: ChainId,
}

impl Connection {
    /// Create a new connection instance.
    pub fn new(address: Address, chain_id: ChainId) -> Self {
        Self { address, chain_id }
    }
}
