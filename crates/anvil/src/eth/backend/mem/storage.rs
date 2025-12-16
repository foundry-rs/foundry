//! In-memory blockchain storage
use crate::eth::{
    backend::{
        db::{
            MaybeFullDatabase, SerializableBlock, SerializableHistoricalStates,
            SerializableTransaction, StateDb,
        },
        env::Env,
        mem::cache::DiskStateCache,
    },
    pool::transactions::PoolTransaction,
};
use alloy_consensus::{Header, constants::EMPTY_WITHDRAWALS};
use alloy_eips::eip7685::EMPTY_REQUESTS_HASH;
use alloy_primitives::{
    B256, Bytes, U256,
    map::{B256HashMap, HashMap},
};
use alloy_rpc_types::{
    BlockId, BlockNumberOrTag, TransactionInfo as RethTransactionInfo,
    trace::{
        otterscan::{InternalOperation, OperationType},
        parity::LocalizedTransactionTrace,
    },
};
use anvil_core::eth::{
    block::{Block, create_block},
    transaction::{MaybeImpersonatedTransaction, TransactionInfo},
};
use foundry_evm::{
    backend::MemDb,
    traces::{CallKind, ParityTraceBuilder, TracingInspectorConfig},
};
use foundry_primitives::{FoundryReceiptEnvelope, FoundryTxReceipt};
use parking_lot::RwLock;
use revm::{context::Block as RevmBlock, primitives::hardfork::SpecId};
use std::{collections::VecDeque, fmt, path::PathBuf, sync::Arc, time::Duration};
// use yansi::Paint;

// === various limits in number of blocks ===

pub const DEFAULT_HISTORY_LIMIT: usize = 500;
const MIN_HISTORY_LIMIT: usize = 10;
// 1hr of up-time at lowest 1s interval
const MAX_ON_DISK_HISTORY_LIMIT: usize = 3_600;

/// Represents the complete state of single block
pub struct InMemoryBlockStates {
    /// The states at a certain block
    states: B256HashMap<StateDb>,
    /// states which data is moved to disk
    on_disk_states: B256HashMap<StateDb>,
    /// How many states to store at most
    in_memory_limit: usize,
    /// minimum amount of states we keep in memory
    min_in_memory_limit: usize,
    /// maximum amount of states we keep on disk
    ///
    /// Limiting the states will prevent disk blow up, especially in interval mining mode
    max_on_disk_limit: usize,
    /// the oldest states written to disk
    oldest_on_disk: VecDeque<B256>,
    /// all states present, used to enforce `in_memory_limit`
    present: VecDeque<B256>,
    /// Stores old states on disk
    disk_cache: DiskStateCache,
}

impl InMemoryBlockStates {
    /// Creates a new instance with limited slots
    pub fn new(in_memory_limit: usize, on_disk_limit: usize) -> Self {
        Self {
            states: Default::default(),
            on_disk_states: Default::default(),
            in_memory_limit,
            min_in_memory_limit: in_memory_limit.min(MIN_HISTORY_LIMIT),
            max_on_disk_limit: on_disk_limit,
            oldest_on_disk: Default::default(),
            present: Default::default(),
            disk_cache: Default::default(),
        }
    }

    /// Configures no disk caching
    pub fn memory_only(mut self) -> Self {
        self.max_on_disk_limit = 0;
        self
    }

    /// Configures the path on disk where the states will cached.
    pub fn disk_path(mut self, path: PathBuf) -> Self {
        self.disk_cache = self.disk_cache.with_path(path);
        self
    }

    /// This modifies the `limit` what to keep stored in memory.
    ///
    /// This will ensure the new limit adjusts based on the block time.
    /// The lowest blocktime is 1s which should increase the limit slightly
    pub fn update_interval_mine_block_time(&mut self, block_time: Duration) {
        let block_time = block_time.as_secs();
        // for block times lower than 2s we increase the mem limit since we're mining _small_ blocks
        // very fast
        // this will gradually be decreased once the max limit was reached
        if block_time <= 2 {
            self.in_memory_limit = DEFAULT_HISTORY_LIMIT * 3;
            self.enforce_limits();
        }
    }

    /// Returns true if only memory caching is supported.
    fn is_memory_only(&self) -> bool {
        self.max_on_disk_limit == 0
    }

    /// Inserts a new (hash -> state) pair
    ///
    /// When the configured limit for the number of states that can be stored in memory is reached,
    /// the oldest state is removed.
    ///
    /// Since we keep a snapshot of the entire state as history, the size of the state will increase
    /// with the transactions processed. To counter this, we gradually decrease the cache limit with
    /// the number of states/blocks until we reached the `min_limit`.
    ///
    /// When a state that was previously written to disk is requested, it is simply read from disk.
    pub fn insert(&mut self, hash: B256, state: StateDb) {
        if !self.is_memory_only() && self.present.len() >= self.in_memory_limit {
            // once we hit the max limit we gradually decrease it
            self.in_memory_limit =
                self.in_memory_limit.saturating_sub(1).max(self.min_in_memory_limit);
        }

        self.enforce_limits();

        self.states.insert(hash, state);
        self.present.push_back(hash);
    }

    /// Enforces configured limits
    fn enforce_limits(&mut self) {
        // enforce memory limits
        while self.present.len() >= self.in_memory_limit {
            // evict the oldest block
            if let Some((hash, mut state)) = self
                .present
                .pop_front()
                .and_then(|hash| self.states.remove(&hash).map(|state| (hash, state)))
            {
                // only write to disk if supported
                if !self.is_memory_only() {
                    let state_snapshot = state.0.clear_into_state_snapshot();
                    self.disk_cache.write(hash, state_snapshot);
                    self.on_disk_states.insert(hash, state);
                    self.oldest_on_disk.push_back(hash);
                }
            }
        }

        // enforce on disk limit and purge the oldest state cached on disk
        while !self.is_memory_only() && self.oldest_on_disk.len() >= self.max_on_disk_limit {
            // evict the oldest block
            if let Some(hash) = self.oldest_on_disk.pop_front() {
                self.on_disk_states.remove(&hash);
                self.disk_cache.remove(hash);
            }
        }
    }

    /// Returns the in-memory state for the given `hash` if present
    pub fn get_state(&self, hash: &B256) -> Option<&StateDb> {
        self.states.get(hash)
    }

    /// Returns on-disk state for the given `hash` if present
    pub fn get_on_disk_state(&mut self, hash: &B256) -> Option<&StateDb> {
        if let Some(state) = self.on_disk_states.get_mut(hash)
            && let Some(cached) = self.disk_cache.read(*hash)
        {
            state.init_from_state_snapshot(cached);
            return Some(state);
        }

        None
    }

    /// Sets the maximum number of stats we keep in memory
    pub fn set_cache_limit(&mut self, limit: usize) {
        self.in_memory_limit = limit;
    }

    /// Clears all entries
    pub fn clear(&mut self) {
        self.states.clear();
        self.on_disk_states.clear();
        self.present.clear();
        for on_disk in std::mem::take(&mut self.oldest_on_disk) {
            self.disk_cache.remove(on_disk)
        }
    }

    /// Serialize all states to a list of serializable historical states
    pub fn serialized_states(&mut self) -> SerializableHistoricalStates {
        // Get in-memory states
        let mut states = self
            .states
            .iter_mut()
            .map(|(hash, state)| (*hash, state.serialize_state()))
            .collect::<Vec<_>>();

        // Get on-disk state snapshots
        self.on_disk_states.iter().for_each(|(hash, _)| {
            if let Some(state_snapshot) = self.disk_cache.read(*hash) {
                states.push((*hash, state_snapshot));
            }
        });

        SerializableHistoricalStates::new(states)
    }

    /// Load states from serialized data
    pub fn load_states(&mut self, states: SerializableHistoricalStates) {
        for (hash, state_snapshot) in states {
            let mut state_db = StateDb::new(MemDb::default());
            state_db.init_from_state_snapshot(state_snapshot);
            self.insert(hash, state_db);
        }
    }
}

impl fmt::Debug for InMemoryBlockStates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InMemoryBlockStates")
            .field("in_memory_limit", &self.in_memory_limit)
            .field("min_in_memory_limit", &self.min_in_memory_limit)
            .field("max_on_disk_limit", &self.max_on_disk_limit)
            .field("oldest_on_disk", &self.oldest_on_disk)
            .field("present", &self.present)
            .finish_non_exhaustive()
    }
}

impl Default for InMemoryBlockStates {
    fn default() -> Self {
        // enough in memory to store `DEFAULT_HISTORY_LIMIT` blocks in memory
        Self::new(DEFAULT_HISTORY_LIMIT, MAX_ON_DISK_HISTORY_LIMIT)
    }
}

/// Stores the blockchain data (blocks, transactions)
#[derive(Clone, Debug)]
pub struct BlockchainStorage {
    /// all stored blocks (block hash -> block)
    pub blocks: B256HashMap<Block>,
    /// mapping from block number -> block hash
    pub hashes: HashMap<u64, B256>,
    /// The current best hash
    pub best_hash: B256,
    /// The current best block number
    pub best_number: u64,
    /// genesis hash of the chain
    pub genesis_hash: B256,
    /// Mapping from the transaction hash to a tuple containing the transaction as well as the
    /// transaction receipt
    pub transactions: B256HashMap<MinedTransaction>,
    /// The total difficulty of the chain until this block
    pub total_difficulty: U256,
}

impl BlockchainStorage {
    /// Creates a new storage with a genesis block
    pub fn new(
        env: &Env,
        spec_id: SpecId,
        base_fee: Option<u64>,
        timestamp: u64,
        genesis_number: u64,
    ) -> Self {
        let is_shanghai = spec_id >= SpecId::SHANGHAI;
        let is_cancun = spec_id >= SpecId::CANCUN;
        let is_prague = spec_id >= SpecId::PRAGUE;

        // create a dummy genesis block
        let header = Header {
            timestamp,
            base_fee_per_gas: base_fee,
            gas_limit: env.evm_env.block_env.gas_limit,
            beneficiary: env.evm_env.block_env.beneficiary,
            difficulty: env.evm_env.block_env.difficulty,
            blob_gas_used: env.evm_env.block_env.blob_excess_gas_and_price.as_ref().map(|_| 0),
            excess_blob_gas: env.evm_env.block_env.blob_excess_gas(),
            number: genesis_number,
            parent_beacon_block_root: is_cancun.then_some(Default::default()),
            withdrawals_root: is_shanghai.then_some(EMPTY_WITHDRAWALS),
            requests_hash: is_prague.then_some(EMPTY_REQUESTS_HASH),
            ..Default::default()
        };
        let block = create_block(header, Vec::<MaybeImpersonatedTransaction>::new());
        let genesis_hash = block.header.hash_slow();
        let best_hash = genesis_hash;
        let best_number = genesis_number;

        let mut blocks = B256HashMap::default();
        blocks.insert(genesis_hash, block);

        let mut hashes = HashMap::default();
        hashes.insert(best_number, genesis_hash);
        Self {
            blocks,
            hashes,
            best_hash,
            best_number,
            genesis_hash,
            transactions: Default::default(),
            total_difficulty: Default::default(),
        }
    }

    pub fn forked(block_number: u64, block_hash: B256, total_difficulty: U256) -> Self {
        let mut hashes = HashMap::default();
        hashes.insert(block_number, block_hash);

        Self {
            blocks: B256HashMap::default(),
            hashes,
            best_hash: block_hash,
            best_number: block_number,
            genesis_hash: Default::default(),
            transactions: Default::default(),
            total_difficulty,
        }
    }

    /// Unwind the chain state back to the given block in storage.
    ///
    /// The block identified by `block_number` and `block_hash` is __non-inclusive__, i.e. it will
    /// remain in the state.
    pub fn unwind_to(&mut self, block_number: u64, block_hash: B256) -> Vec<Block> {
        let mut removed = vec![];
        let best_num: u64 = self.best_number;
        for i in (block_number + 1)..=best_num {
            if let Some(hash) = self.hashes.get(&i).copied() {
                // First remove the block's transactions while the mappings still exist
                self.remove_block_transactions_by_number(i);

                // Now remove the block from storage (may already be empty of txs) and drop mapping
                if let Some(block) = self.blocks.remove(&hash) {
                    removed.push(block);
                }
                self.hashes.remove(&i);
            }
        }
        self.best_hash = block_hash;
        self.best_number = block_number;
        removed
    }

    pub fn empty() -> Self {
        Self {
            blocks: Default::default(),
            hashes: Default::default(),
            best_hash: Default::default(),
            best_number: Default::default(),
            genesis_hash: Default::default(),
            transactions: Default::default(),
            total_difficulty: Default::default(),
        }
    }

    /// Removes all stored transactions for the given block number
    pub fn remove_block_transactions_by_number(&mut self, num: u64) {
        if let Some(hash) = self.hashes.get(&num).copied() {
            self.remove_block_transactions(hash);
        }
    }

    /// Removes all stored transactions for the given block hash
    pub fn remove_block_transactions(&mut self, block_hash: B256) {
        if let Some(block) = self.blocks.get_mut(&block_hash) {
            for tx in &block.body.transactions {
                self.transactions.remove(&tx.hash());
            }
            block.body.transactions.clear();
        }
    }
}

impl BlockchainStorage {
    /// Returns the hash for [BlockNumberOrTag]
    pub fn hash(&self, number: BlockNumberOrTag) -> Option<B256> {
        let slots_in_an_epoch = 32;
        match number {
            BlockNumberOrTag::Latest => Some(self.best_hash),
            BlockNumberOrTag::Earliest => Some(self.genesis_hash),
            BlockNumberOrTag::Pending => None,
            BlockNumberOrTag::Number(num) => self.hashes.get(&num).copied(),
            BlockNumberOrTag::Safe => {
                if self.best_number > (slots_in_an_epoch) {
                    self.hashes.get(&(self.best_number - (slots_in_an_epoch))).copied()
                } else {
                    Some(self.genesis_hash) // treat the genesis block as safe "by definition"
                }
            }
            BlockNumberOrTag::Finalized => {
                if self.best_number > (slots_in_an_epoch * 2) {
                    self.hashes.get(&(self.best_number - (slots_in_an_epoch * 2))).copied()
                } else {
                    Some(self.genesis_hash)
                }
            }
        }
    }

    pub fn serialized_blocks(&self) -> Vec<SerializableBlock> {
        self.blocks.values().map(|block| block.clone().into()).collect()
    }

    pub fn serialized_transactions(&self) -> Vec<SerializableTransaction> {
        self.transactions.values().map(|tx: &MinedTransaction| tx.clone().into()).collect()
    }

    /// Deserialize and add all blocks data to the backend storage
    pub fn load_blocks(&mut self, serializable_blocks: Vec<SerializableBlock>) {
        for serializable_block in &serializable_blocks {
            let block: Block = serializable_block.clone().into();
            let block_hash = block.header.hash_slow();
            let block_number = block.header.number;
            self.blocks.insert(block_hash, block);
            self.hashes.insert(block_number, block_hash);
        }
    }

    /// Deserialize and add all blocks data to the backend storage
    pub fn load_transactions(&mut self, serializable_transactions: Vec<SerializableTransaction>) {
        for serializable_transaction in &serializable_transactions {
            let transaction: MinedTransaction = serializable_transaction.clone().into();
            self.transactions.insert(transaction.info.transaction_hash, transaction);
        }
    }
}

/// A simple in-memory blockchain
#[derive(Clone, Debug)]
pub struct Blockchain {
    /// underlying storage that supports concurrent reads
    pub storage: Arc<RwLock<BlockchainStorage>>,
}

impl Blockchain {
    /// Creates a new storage with a genesis block
    pub fn new(
        env: &Env,
        spec_id: SpecId,
        base_fee: Option<u64>,
        timestamp: u64,
        genesis_number: u64,
    ) -> Self {
        Self {
            storage: Arc::new(RwLock::new(BlockchainStorage::new(
                env,
                spec_id,
                base_fee,
                timestamp,
                genesis_number,
            ))),
        }
    }

    pub fn forked(block_number: u64, block_hash: B256, total_difficulty: U256) -> Self {
        Self {
            storage: Arc::new(RwLock::new(BlockchainStorage::forked(
                block_number,
                block_hash,
                total_difficulty,
            ))),
        }
    }

    /// returns the header hash of given block
    pub fn hash(&self, id: BlockId) -> Option<B256> {
        match id {
            BlockId::Hash(h) => Some(h.block_hash),
            BlockId::Number(num) => self.storage.read().hash(num),
        }
    }

    pub fn get_block_by_hash(&self, hash: &B256) -> Option<Block> {
        self.storage.read().blocks.get(hash).cloned()
    }

    pub fn get_transaction_by_hash(&self, hash: &B256) -> Option<MinedTransaction> {
        self.storage.read().transactions.get(hash).cloned()
    }

    /// Returns the total number of blocks
    pub fn blocks_count(&self) -> usize {
        self.storage.read().blocks.len()
    }
}

/// Represents the outcome of mining a new block
#[derive(Clone, Debug)]
pub struct MinedBlockOutcome {
    /// The block that was mined
    pub block_number: u64,
    /// All transactions included in the block
    pub included: Vec<Arc<PoolTransaction>>,
    /// All transactions that were attempted to be included but were invalid at the time of
    /// execution
    pub invalid: Vec<Arc<PoolTransaction>>,
}

/// Container type for a mined transaction
#[derive(Clone, Debug)]
pub struct MinedTransaction {
    pub info: TransactionInfo,
    pub receipt: FoundryReceiptEnvelope,
    pub block_hash: B256,
    pub block_number: u64,
}

impl MinedTransaction {
    /// Returns the traces of the transaction for `trace_transaction`
    pub fn parity_traces(&self) -> Vec<LocalizedTransactionTrace> {
        ParityTraceBuilder::new(
            self.info.traces.clone(),
            None,
            TracingInspectorConfig::default_parity(),
        )
        .into_localized_transaction_traces(RethTransactionInfo {
            hash: Some(self.info.transaction_hash),
            index: Some(self.info.transaction_index),
            block_hash: Some(self.block_hash),
            block_number: Some(self.block_number),
            base_fee: None,
        })
    }

    pub fn ots_internal_operations(&self) -> Vec<InternalOperation> {
        self.info
            .traces
            .iter()
            .filter_map(|node| {
                let r#type = match node.trace.kind {
                    _ if node.is_selfdestruct() => OperationType::OpSelfDestruct,
                    CallKind::Call if !node.trace.value.is_zero() => OperationType::OpTransfer,
                    CallKind::Create => OperationType::OpCreate,
                    CallKind::Create2 => OperationType::OpCreate2,
                    _ => return None,
                };
                let mut from = node.trace.caller;
                let mut to = node.trace.address;
                let mut value = node.trace.value;
                if node.is_selfdestruct() {
                    from = node.trace.address;
                    to = node.trace.selfdestruct_refund_target.unwrap_or_default();
                    value = node.trace.selfdestruct_transferred_value.unwrap_or_default();
                }
                Some(InternalOperation { r#type, from, to, value })
            })
            .collect()
    }
}

/// Intermediary Anvil representation of a receipt
#[derive(Clone, Debug)]
pub struct MinedTransactionReceipt {
    /// The actual json rpc receipt object
    pub inner: FoundryTxReceipt,
    /// Output data for the transaction
    pub out: Option<Bytes>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eth::backend::db::Db;
    use alloy_primitives::{Address, hex};
    use alloy_rlp::Decodable;
    use foundry_primitives::FoundryTxEnvelope;
    use revm::{database::DatabaseRef, state::AccountInfo};

    #[test]
    fn test_interval_update() {
        let mut storage = InMemoryBlockStates::default();
        storage.update_interval_mine_block_time(Duration::from_secs(1));
        assert_eq!(storage.in_memory_limit, DEFAULT_HISTORY_LIMIT * 3);
    }

    #[test]
    fn test_init_state_limits() {
        let mut storage = InMemoryBlockStates::default();
        assert_eq!(storage.in_memory_limit, DEFAULT_HISTORY_LIMIT);
        assert_eq!(storage.min_in_memory_limit, MIN_HISTORY_LIMIT);
        assert_eq!(storage.max_on_disk_limit, MAX_ON_DISK_HISTORY_LIMIT);

        storage = storage.memory_only();
        assert!(storage.is_memory_only());

        storage = InMemoryBlockStates::new(1, 0);
        assert!(storage.is_memory_only());
        assert_eq!(storage.in_memory_limit, 1);
        assert_eq!(storage.min_in_memory_limit, 1);
        assert_eq!(storage.max_on_disk_limit, 0);

        storage = InMemoryBlockStates::new(1, 2);
        assert!(!storage.is_memory_only());
        assert_eq!(storage.in_memory_limit, 1);
        assert_eq!(storage.min_in_memory_limit, 1);
        assert_eq!(storage.max_on_disk_limit, 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn can_read_write_cached_state() {
        let mut storage = InMemoryBlockStates::new(1, MAX_ON_DISK_HISTORY_LIMIT);
        let one = B256::from(U256::from(1));
        let two = B256::from(U256::from(2));

        let mut state = MemDb::default();
        let addr = Address::random();
        let info = AccountInfo::from_balance(U256::from(1337));
        state.insert_account(addr, info);
        storage.insert(one, StateDb::new(state));
        storage.insert(two, StateDb::new(MemDb::default()));

        // wait for files to be flushed
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        assert_eq!(storage.on_disk_states.len(), 1);
        assert!(storage.on_disk_states.contains_key(&one));

        let loaded = storage.get_on_disk_state(&one).unwrap();

        let acc = loaded.basic_ref(addr).unwrap().unwrap();
        assert_eq!(acc.balance, U256::from(1337u64));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn can_decrease_state_cache_size() {
        let limit = 15;
        let mut storage = InMemoryBlockStates::new(limit, MAX_ON_DISK_HISTORY_LIMIT);

        let num_states = 30;
        for idx in 0..num_states {
            let mut state = MemDb::default();
            let hash = B256::from(U256::from(idx));
            let addr = Address::from_word(hash);
            let balance = (idx * 2) as u64;
            let info = AccountInfo::from_balance(U256::from(balance));
            state.insert_account(addr, info);
            storage.insert(hash, StateDb::new(state));
        }

        // wait for files to be flushed
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        let on_disk_states_len = num_states - storage.min_in_memory_limit;

        assert_eq!(storage.on_disk_states.len(), on_disk_states_len);
        assert_eq!(storage.present.len(), storage.min_in_memory_limit);

        for idx in 0..num_states {
            let hash = B256::from(U256::from(idx));
            let addr = Address::from_word(hash);

            let loaded = if idx < on_disk_states_len {
                storage.get_on_disk_state(&hash).unwrap()
            } else {
                storage.get_state(&hash).unwrap()
            };

            let acc = loaded.basic_ref(addr).unwrap().unwrap();
            let balance = (idx * 2) as u64;
            assert_eq!(acc.balance, U256::from(balance));
        }
    }

    // verifies that blocks and transactions in BlockchainStorage remain the same when dumped and
    // reloaded
    #[test]
    fn test_storage_dump_reload_cycle() {
        let mut dump_storage = BlockchainStorage::empty();

        let header = Header { gas_limit: 123456, ..Default::default() };
        let bytes_first = &mut &hex::decode("f86b02843b9aca00830186a094d3e8763675e4c425df46cc3b5c0f6cbdac39604687038d7ea4c68000802ba00eb96ca19e8a77102767a41fc85a36afd5c61ccb09911cec5d3e86e193d9c5aea03a456401896b1b6055311536bf00a718568c744d8c1f9df59879e8350220ca18").unwrap()[..];
        let tx: MaybeImpersonatedTransaction =
            FoundryTxEnvelope::decode(&mut &bytes_first[..]).unwrap().into();
        let block = create_block(header.clone(), vec![tx.clone()]);
        let block_hash = block.header.hash_slow();
        dump_storage.blocks.insert(block_hash, block);

        let serialized_blocks = dump_storage.serialized_blocks();
        let serialized_transactions = dump_storage.serialized_transactions();

        let mut load_storage = BlockchainStorage::empty();

        load_storage.load_blocks(serialized_blocks);
        load_storage.load_transactions(serialized_transactions);

        let loaded_block = load_storage.blocks.get(&block_hash).unwrap();
        assert_eq!(loaded_block.header.gas_limit, { header.gas_limit });
        let loaded_tx = loaded_block.body.transactions.first().unwrap();
        assert_eq!(loaded_tx, &tx);
    }
}
