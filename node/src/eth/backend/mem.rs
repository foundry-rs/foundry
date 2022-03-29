//! In memory blockchain backend

use crate::eth::backend::db::Db;
use ethers::{
    prelude::{Block, BlockNumber, Transaction, TransactionReceipt, TxHash, H256, U256, U64},
    types::BlockId,
};
use foundry_evm::{
    executor::DatabaseRef,
    revm::{db::CacheDB, Database, Env},
};
use parking_lot::RwLock;
use std::{collections::HashMap, sync::Arc};

/// Stores the blockchain data (blocks, transactions)
#[derive(Clone, Default)]
struct BlockchainStorage {
    /// all stored blocks (block hash -> block)
    blocks: HashMap<H256, Block<TxHash>>,
    /// mapping from block number -> block hash
    hashes: HashMap<U64, H256>,
    /// The current best hash
    best_hash: H256,
    /// The current best block number
    best_number: U64,
    /// last finalized block hash
    finalized_hash: H256,
    /// last finalized block number
    finalized_number: U64,
    /// genesis hash of the chain
    genesis_hash: H256,
    /// Mapping from the transaction hash to a tuple containing the transaction as well as the
    /// transaction receipt
    transactions: HashMap<TxHash, (Transaction, TransactionReceipt)>,
}

impl BlockchainStorage {
    /// Returns the hash for [BlockNumber]
    pub fn hash(&self, number: BlockNumber) -> Option<H256> {
        match number {
            BlockNumber::Latest => Some(self.best_hash),
            BlockNumber::Earliest => Some(self.genesis_hash),
            BlockNumber::Pending => None,
            BlockNumber::Number(num) => self.hashes.get(&num).copied(),
        }
    }
}

/// A simple in-memory blockchain
#[derive(Clone, Default)]
pub struct Blockchain {
    /// underlying storage that supports concurrent reads
    storage: Arc<RwLock<BlockchainStorage>>,
}

impl Blockchain {
    /// returns the header hash of given block
    pub fn hash(&self, id: BlockId) -> Option<H256> {
        match id {
            BlockId::Hash(h) => Some(h),
            BlockId::Number(num) => self.storage.read().hash(num),
        }
    }

    /// Returns the total number of blocks
    pub fn blocks_count(&self) -> usize {
        self.storage.read().blocks.len()
    }
}

/// Gives access to the [revm::Database]
#[derive(Clone)]
pub struct Backend {
    /// access to revm's database related operations
    /// This stores the actual state of the blockchain
    /// Supports concurrent reads
    db: Arc<RwLock<dyn Db>>,
    /// stores all block related data in memory
    blockchain: Blockchain,
    /// env data of the chain
    env: Arc<RwLock<Env>>,
}

impl Backend {
    /// Create a new instance of in-mem backend.
    pub fn new(db: Arc<RwLock<dyn Db>>, env: Arc<RwLock<Env>>) -> Self {
        Self { db, blockchain: Blockchain::default(), env }
    }

    /// Creates a new empty blockchain backend
    pub fn empty(env: Arc<RwLock<Env>>) -> Self {
        let db = CacheDB::default();
        Self::new(Arc::new(RwLock::new(db)), env)
    }

    /// The env data of the blockchain
    pub fn env(&self) -> &Arc<RwLock<Env>> {
        &self.env
    }

    /// Returns the current best hash of the chain
    pub fn best_hash(&self) -> H256 {
        self.blockchain.storage.read().best_hash
    }

    /// Returns the current best number of the chain
    pub fn best_number(&self) -> U64 {
        self.blockchain.storage.read().best_number
    }

    pub fn gas_limit(&self) -> U256 {
        // TODO make this a separate value?
        self.env().read().block.gas_limit
    }
}
