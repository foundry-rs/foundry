//! In-memory blockchain storage
use crate::{eth::backend::time::duration_since_unix_epoch, mem::MinedTransaction};
use anvil_core::eth::block::{Block, PartialHeader};
use ethers::prelude::{BlockId, BlockNumber, H256, H256 as TxHash, U64};
use parking_lot::RwLock;
use std::{collections::HashMap, sync::Arc};

/// Stores the blockchain data (blocks, transactions)
#[derive(Clone)]
pub struct BlockchainStorage {
    /// all stored blocks (block hash -> block)
    pub blocks: HashMap<H256, Block>,
    /// mapping from block number -> block hash
    pub hashes: HashMap<U64, H256>,
    /// The current best hash
    pub best_hash: H256,
    /// The current best block number
    pub best_number: U64,
    /// genesis hash of the chain
    pub genesis_hash: H256,
    /// Mapping from the transaction hash to a tuple containing the transaction as well as the
    /// transaction receipt
    pub transactions: HashMap<TxHash, MinedTransaction>,
}

impl BlockchainStorage {
    #[allow(unused)]
    pub fn empty() -> Self {
        Self {
            blocks: Default::default(),
            hashes: Default::default(),
            best_hash: Default::default(),
            best_number: Default::default(),
            genesis_hash: Default::default(),
            transactions: Default::default(),
        }
    }
}

impl Default for BlockchainStorage {
    fn default() -> Self {
        // create a dummy genesis block
        let partial_header = PartialHeader {
            timestamp: duration_since_unix_epoch().as_secs(),
            ..Default::default()
        };
        let block = Block::new(partial_header, vec![], vec![]);
        let genesis_hash = block.header.hash();
        let best_hash = genesis_hash;
        let best_number: U64 = 0u64.into();

        Self {
            blocks: HashMap::from([(genesis_hash, block)]),
            hashes: HashMap::from([(best_number, genesis_hash)]),
            best_hash,
            best_number,
            genesis_hash,
            transactions: Default::default(),
        }
    }
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
    pub storage: Arc<RwLock<BlockchainStorage>>,
}

impl Blockchain {
    pub fn forked(block_number: u64, block_hash: H256) -> Self {
        let storage = BlockchainStorage {
            blocks: Default::default(),
            hashes: HashMap::from([(block_number.into(), block_hash)]),
            best_hash: block_hash,
            best_number: block_number.into(),
            genesis_hash: Default::default(),
            transactions: Default::default(),
        };
        Self { storage: Arc::new(RwLock::new(storage)) }
    }

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
