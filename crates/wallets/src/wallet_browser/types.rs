use alloy_primitives::{Address, ChainId, TxHash};
use alloy_rpc_types::TransactionRequest;
use serde::{Deserialize, Serialize};

/// Wallet connection information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletConnection {
    pub address: Address,
    pub chain_id: ChainId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_name: Option<String>,
}

/// Browser-specific transaction wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserTransaction {
    /// Unique ID for tracking in the browser
    pub id: String,
    /// Standard Alloy transaction request
    #[serde(flatten)]
    pub request: TransactionRequest,
}

/// Transaction response from the browser
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionResponse {
    pub id: String,
    pub hash: Option<TxHash>,
    pub error: Option<String>,
}
