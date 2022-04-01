//! In memory blockchain backend

use crate::eth::{backend::db::Db, pool::transactions::PoolTransaction};
use ethers::{
    prelude::{Block, BlockNumber, Transaction, TransactionReceipt, TxHash, H256, U256, U64},
    types::BlockId,
};
use forge_node_core::eth::{block::Header, receipt::Log, transaction::PendingTransaction};
use foundry_evm::{
    executor::DatabaseRef,
    revm::{self, db::CacheDB, BlockEnv, CfgEnv, Database, Env, Return, TransactOut},
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

    /// Mines a new block and stores it.
    ///
    /// this will execute all transaction in the order they come in and return all the markers they
    /// provide .
    ///
    /// TODO(mattsse): currently we're assuming transaction is valid, needs an additional validation
    /// step: gas limit, fee
    pub fn mine_block(&self, transactions: Vec<Arc<PoolTransaction>>) {
        let env = self.env.write();
        let mut db = self.db.write();
        let _miner = TransactionMiner {
            db: &mut *db,
            pending: transactions.into_iter(),
            block_env: env.block.clone(),
            cfg_env: env.cfg.clone(),
        };

        // TODO update env
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

/// Represents a transacted(on the DB) transaction
struct MinedTransaction {
    transaction: Arc<PoolTransaction>,
    exit: Return,
    out: TransactOut,
    gas: u64,
    logs: Vec<Log>,
}

/// An executer for a series of transactions
struct TransactionMiner<Db> {
    db: Db,
    pending: std::vec::IntoIter<Arc<PoolTransaction>>,
    block_env: BlockEnv,
    cfg_env: CfgEnv,
}

impl<DB: Db> TransactionMiner<DB> {
    fn mine_block(self) {
        let mut transactions = Vec::new();
        // let mut statuses = Vec::new();
        let mut receipts = Vec::new();
        // let mut logs_bloom = Bloom::default();
        // let mut cumulative_gas_used = U256::zero();

        for (_idx, tx) in self.enumerate() {
            let MinedTransaction { transaction, exit: _, out: _, gas: _, logs } = tx;
            transactions.push(transaction.pending_transaction.transaction.clone());
            receipts.push(logs.clone());
        }

        let _ommers: Vec<Header> = Vec::new();
        // let receipts_root = trie::ordered_trie_root(receipts.iter().map(|r| rlp::encode(r)));
    }

    fn env_for(&self, tx: &PendingTransaction) -> Env {
        Env { cfg: self.cfg_env.clone(), block: self.block_env.clone(), tx: tx.to_revm_tx_env() }
    }
}

impl<DB: Db> Iterator for TransactionMiner<DB> {
    type Item = MinedTransaction;

    fn next(&mut self) -> Option<Self::Item> {
        let transaction = self.pending.next()?;

        let mut evm = revm::EVM::new();
        evm.env = self.env_for(&transaction.pending_transaction);
        evm.database(&mut self.db);

        // transact and commit the transaction
        let (exit, out, gas, logs) = evm.transact_commit();

        Some(MinedTransaction {
            transaction,
            exit,
            out,
            gas,
            logs: logs.into_iter().map(Into::into).collect(),
        })
    }
}
