use alloy_primitives::{Address, ChainId, TxHash};
use alloy_rpc_types::TransactionRequest;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WalletConnection {
    pub address: Address,
    pub chain_id: ChainId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BrowserTransaction {
    pub id: Uuid,
    #[serde(flatten)]
    pub request: TransactionRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TransactionResponse {
    pub id: Uuid,
    pub hash: Option<TxHash>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AccountUpdate {
    pub address: Option<Address>,
    pub chain_id: Option<ChainId>,
}
