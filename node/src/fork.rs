//! Support for forking off another client

use ethers::{
    prelude::{Http, Provider},
    types::H256,
};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ForkInfo {
    pub eth_rpc_url: String,
    pub block_number: u64,
    pub block_hash: H256,
    // TODO make provider agnostic
    pub provider: Arc<Provider<Http>>,
}
