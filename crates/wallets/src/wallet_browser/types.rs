use alloy_primitives::{Address, ChainId, TxHash};
use alloy_rpc_types::TransactionRequest;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WalletConnection {
    pub address: Address,
    pub chain_id: ChainId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BrowserTransaction {
    pub id: String,
    #[serde(flatten)]
    pub request: TransactionRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TransactionResponse {
    pub id: String,
    pub hash: Option<TxHash>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AccountUpdate {
    pub address: Option<Address>,
    pub chain_id: Option<ChainId>,
}
