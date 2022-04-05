//! In memory blockchain backend

use crate::eth::{
    backend::{db::Db, executor::TransactionExecutor},
    pool::transactions::PoolTransaction,
};
use ethers::{
    prelude::{BlockNumber, TxHash, H256, U256, U64},
    types::BlockId,
};

use forge_node_core::eth::{
    block::{Block, BlockInfo},
    receipt::TypedReceipt,
    transaction::TransactionInfo,
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
    blocks: HashMap<H256, Block>,
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
    transactions: HashMap<TxHash, (TransactionInfo, TypedReceipt)>,
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

    /// Mines a new block and stores it.
    ///
    /// this will execute all transaction in the order they come in and return all the markers they
    /// provide.
    ///
    /// TODO(mattsse): currently we're assuming all transactions are valid:
    ///  needs an additional validation step: gas limit, fee
    pub fn mine_block(&self, pool_transactions: Vec<Arc<PoolTransaction>>) {
        // acquire all locks
        let mut env = self.env.write();
        let mut db = self.db.write();
        let mut storage = self.blockchain.storage.write();

        let executor = TransactionExecutor {
            db: &mut *db,
            pending: pool_transactions.into_iter(),
            block_env: env.block.clone(),
            cfg_env: env.cfg.clone(),
            parent_hash: storage.finalized_hash,
        };

        let BlockInfo { block, transactions, receipts } = executor.create_block();

        // update block metadata
        storage.finalized_number = env.block.number.as_u64().into();
        env.block.number = env.block.number.saturating_add(U256::one());
        storage.best_number = env.block.number.as_u64().into();

        storage.finalized_hash = block.header.hash();
        storage.best_hash = storage.finalized_hash;

        let hash = storage.finalized_hash;
        let number = storage.finalized_number;
        storage.blocks.insert(hash, block);
        storage.hashes.insert(number, hash);

        // insert all transactions
        for (tx, receipt) in transactions.into_iter().zip(receipts) {
            storage.transactions.insert(tx.transaction_hash, (tx, receipt));
        }
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
