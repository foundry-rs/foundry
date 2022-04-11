//! Support for forking off another client

use std::collections::HashMap;
use ethers::{
    prelude::{Http, Provider},
    types::H256,
};
use std::sync::Arc;

use ethers::{
    types::{Address, Block, Log, Transaction, TransactionReceipt},
    utils::{keccak256, rlp},
};
use ethers::types::TxHash;

#[derive(Debug, Clone)]
pub struct ClientFork {
    pub eth_rpc_url: String,
    pub block_number: u64,
    pub block_hash: H256,
    pub storage: ForkedStorage,
    // TODO make provider agnostic
    pub provider: Arc<Provider<Http>>,
}


/// Contains cached state fetched to serve EthApi requests
#[derive(Debug, Clone, Default)]
pub struct ForkedStorage {
    pub blocks: HashMap<H256, Block<TxHash>>,

}