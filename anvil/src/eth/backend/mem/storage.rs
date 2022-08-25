//! In-memory blockchain storage
use crate::eth::{
    backend::{
        db::{MaybeHashDatabase, StateDb},
        mem::cache::DiskStateCache,
    },
    pool::transactions::PoolTransaction,
};
use anvil_core::eth::{
    block::{Block, PartialHeader},
    receipt::TypedReceipt,
    transaction::TransactionInfo,
};
use ethers::{
    prelude::{BlockId, BlockNumber, GethTrace, Trace, H256, H256 as TxHash, U64},
    types::{ActionType, GethDebugTracingOptions, U256},
};
use forge::revm::{Env, Return};
use parking_lot::RwLock;
use std::{
    collections::{HashMap, VecDeque},
    fmt,
    sync::Arc,
};

// === impl DiskStateCache ===

/// Represents the complete state of single block
pub struct InMemoryBlockStates {
    /// The states at a certain block
    states: HashMap<H256, StateDb>,
    /// states which data is moved to disk
    on_disk_states: HashMap<H256, StateDb>,
    /// How many states to store at most
    limit: usize,
    /// all states present, used to enforce `limit`
    present: VecDeque<H256>,
    /// Stores old states on disk
    disk_cache: DiskStateCache,
}

// === impl InMemoryBlockStates ===

impl InMemoryBlockStates {
    /// Creates a new instance with limited slots
    pub fn new(limit: usize) -> Self {
        Self {
            states: Default::default(),
            on_disk_states: Default::default(),
            limit,
            present: Default::default(),
            disk_cache: Default::default(),
        }
    }

    /// Inserts a new (hash -> state) pair
    ///
    /// When the configured limit for the number of states that can be stored in memory is reached,
    /// the oldest state is removed.
    pub fn insert(&mut self, hash: H256, state: StateDb) {
        if self.present.len() >= self.limit {
            // evict the oldest block
            if let Some((hash, mut state)) = self
                .present
                .pop_front()
                .and_then(|hash| self.states.remove(&hash).map(|state| (hash, state)))
            {
                let snapshot = state.0.clear_into_snapshot();
                self.disk_cache.write(hash, &snapshot);
                self.on_disk_states.insert(hash, state);
            }
        }
        self.states.insert(hash, state);
        self.present.push_back(hash);
    }

    /// Returns the state for the given `hash` if present
    pub fn get(&mut self, hash: &H256) -> Option<&StateDb> {
        self.states.get(hash).or_else(|| {
            if let Some(state) = self.on_disk_states.get_mut(hash) {
                if let Some(cached) = self.disk_cache.read(*hash) {
                    state.init_from_snapshot(cached);
                    return Some(state)
                }
            }
            None
        })
    }

    /// Clears all entries
    pub fn clear(&mut self) {
        self.states.clear();
        self.on_disk_states.clear();
        self.present.clear();
    }
}

impl fmt::Debug for InMemoryBlockStates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InMemoryBlockStates")
            .field("limit", &self.limit)
            .field("present", &self.present)
            .finish_non_exhaustive()
    }
}

impl Default for InMemoryBlockStates {
    fn default() -> Self {
        // enough in memory to store 1_000 blocks in memory, this is ~30min of up-time with 1s
        // interval mining mode
        Self::new(1_000)
    }
}

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
    /// Creates a new storage with a genesis block
    pub fn new(env: &Env, base_fee: Option<U256>, timestamp: u64) -> Self {
        // create a dummy genesis block
        let partial_header = PartialHeader {
            timestamp,
            base_fee,
            gas_limit: env.block.gas_limit,
            beneficiary: env.block.coinbase,
            difficulty: env.block.difficulty,
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

    pub fn forked(block_number: u64, block_hash: H256) -> Self {
        BlockchainStorage {
            blocks: Default::default(),
            hashes: HashMap::from([(block_number.into(), block_hash)]),
            best_hash: block_hash,
            best_number: block_number.into(),
            genesis_hash: Default::default(),
            transactions: Default::default(),
        }
    }

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

// === impl BlockchainStorage ===

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
#[derive(Clone)]
pub struct Blockchain {
    /// underlying storage that supports concurrent reads
    pub storage: Arc<RwLock<BlockchainStorage>>,
}

// === impl BlockchainStorage ===

impl Blockchain {
    /// Creates a new storage with a genesis block
    pub fn new(env: &Env, base_fee: Option<U256>, timestamp: u64) -> Self {
        Self { storage: Arc::new(RwLock::new(BlockchainStorage::new(env, base_fee, timestamp))) }
    }

    pub fn forked(block_number: u64, block_hash: H256) -> Self {
        Self { storage: Arc::new(RwLock::new(BlockchainStorage::forked(block_number, block_hash))) }
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

/// Represents the outcome of mining a new block
#[derive(Debug, Clone)]
pub struct MinedBlockOutcome {
    /// The block that was mined
    pub block_number: U64,
    /// All transactions included in the block
    pub included: Vec<Arc<PoolTransaction>>,
    /// All transactions that were attempted to be included but were invalid at the time of
    /// execution
    pub invalid: Vec<Arc<PoolTransaction>>,
}

/// Container type for a mined transaction
#[derive(Debug, Clone)]
pub struct MinedTransaction {
    pub info: TransactionInfo,
    pub receipt: TypedReceipt,
    pub block_hash: H256,
    pub block_number: u64,
}

// === impl MinedTransaction ===

impl MinedTransaction {
    /// Returns the traces of the transaction for `trace_transaction`
    pub fn parity_traces(&self) -> Vec<Trace> {
        let mut traces = Vec::with_capacity(self.info.traces.arena.len());
        for (idx, node) in self.info.traces.arena.iter().cloned().enumerate() {
            let action = node.parity_action();
            let result = node.parity_result();

            let action_type = if node.status() == Return::SelfDestruct {
                ActionType::Suicide
            } else {
                node.kind().into()
            };

            let trace = Trace {
                action,
                result: Some(result),
                trace_address: self.info.trace_address(idx),
                subtraces: node.children.len(),
                transaction_position: Some(self.info.transaction_index as usize),
                transaction_hash: Some(self.info.transaction_hash),
                block_number: self.block_number,
                block_hash: self.block_hash,
                action_type,
                error: None,
            };
            traces.push(trace)
        }

        traces
    }

    pub fn geth_trace(&self, opts: GethDebugTracingOptions) -> GethTrace {
        self.info.traces.geth_trace(opts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eth::backend::db::Db;
    use ethers::{abi::ethereum_types::BigEndianHash, types::Address};
    use forge::revm::{db::DatabaseRef, AccountInfo};
    use foundry_evm::executor::backend::MemDb;

    #[test]
    fn can_read_write_cached_state() {
        let mut storage = InMemoryBlockStates::new(1);
        let one = H256::from_uint(&U256::from(1));
        let two = H256::from_uint(&U256::from(2));

        let mut state = MemDb::default();
        let addr = Address::random();
        let info = AccountInfo::from_balance(1337.into());
        state.insert_account(addr, info);
        storage.insert(one, StateDb::new(state));
        storage.insert(two, StateDb::new(MemDb::default()));

        assert_eq!(storage.on_disk_states.len(), 1);
        assert!(storage.on_disk_states.get(&one).is_some());

        let loaded = storage.get(&one).unwrap();

        let acc = loaded.basic(addr);
        assert_eq!(acc.balance, 1337u64.into());
    }
}
