//! Offline wrapper for ForkedDatabase that prevents RPC calls

use crate::eth::backend::{
    db::{
        Db, MaybeForkedDatabase, MaybeFullDatabase, SerializableBlock, SerializableState,
        SerializableTransaction, StateDb,
    },
    mem::fork_db::ForkedDatabase,
};
use alloy_primitives::{Address, B256, U256, map::HashMap};
use alloy_rpc_types::BlockId;
use foundry_evm::backend::{
    BlockchainDb, DatabaseError, DatabaseResult, RevertStateSnapshotAction, StateSnapshot,
};
use revm::{
    Database, DatabaseCommit,
    bytecode::Bytecode,
    context::BlockEnv,
    database::{DatabaseRef, DbAccount},
    primitives::KECCAK_EMPTY,
    state::AccountInfo,
};

/// A wrapper around ForkedDatabase that operates in offline mode
///
/// This wrapper intercepts all database calls and returns default values
/// for missing data instead of attempting RPC calls to fetch it.
#[derive(Clone, Debug)]
pub struct OfflineForkedDatabase {
    inner: ForkedDatabase,
}

impl OfflineForkedDatabase {
    pub fn new(inner: ForkedDatabase) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> &ForkedDatabase {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut ForkedDatabase {
        &mut self.inner
    }

    pub fn insert_state_snapshot(&mut self) -> U256 {
        self.inner.insert_state_snapshot()
    }
}

impl Database for OfflineForkedDatabase {
    type Error = DatabaseError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        // Try to get from cache first
        match self.inner.database().cache.accounts.get(&address) {
            Some(account) => Ok(Some(account.info.clone())),
            None => {
                // In offline mode, return default account info instead of fetching
                Ok(Some(AccountInfo::default()))
            }
        }
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        // If it's the empty hash, return empty bytecode
        if code_hash == KECCAK_EMPTY {
            return Ok(Bytecode::default());
        }

        // Try to get from cache
        match self.inner.database().cache.contracts.get(&code_hash) {
            Some(code) => Ok(code.clone()),
            None => {
                // In offline mode, return empty bytecode for missing code
                Ok(Bytecode::default())
            }
        }
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        // Try to get from cache first
        if let Some(account) = self.inner.database().cache.accounts.get(&address) {
            if let Some(value) = account.storage.get(&index) {
                return Ok(*value);
            }
        }

        // In offline mode, return zero for missing storage
        Ok(U256::ZERO)
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        // Delegate to inner - block hashes are pre-loaded
        self.inner.block_hash(number)
    }
}

impl DatabaseRef for OfflineForkedDatabase {
    type Error = DatabaseError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        // Try to get from cache first
        match self.inner.database().cache.accounts.get(&address) {
            Some(account) => Ok(Some(account.info.clone())),
            None => {
                // In offline mode, return default account info instead of fetching
                Ok(Some(AccountInfo::default()))
            }
        }
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        // If it's the empty hash, return empty bytecode
        if code_hash == KECCAK_EMPTY {
            return Ok(Bytecode::default());
        }

        // Try to get from cache
        match self.inner.database().cache.contracts.get(&code_hash) {
            Some(code) => Ok(code.clone()),
            None => {
                // In offline mode, return empty bytecode for missing code
                Ok(Bytecode::default())
            }
        }
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        // Try to get from cache first
        if let Some(account) = self.inner.database().cache.accounts.get(&address) {
            if let Some(value) = account.storage.get(&index) {
                return Ok(*value);
            }
        }

        // In offline mode, return zero for missing storage
        Ok(U256::ZERO)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        // Delegate to inner - block hashes are pre-loaded
        self.inner.block_hash_ref(number)
    }
}

impl DatabaseCommit for OfflineForkedDatabase {
    fn commit(&mut self, changes: HashMap<Address, revm::state::Account>) {
        self.inner.commit(changes)
    }
}

// Implement MaybeFullDatabase trait
impl MaybeFullDatabase for OfflineForkedDatabase {
    fn maybe_as_full_db(&self) -> Option<&HashMap<Address, DbAccount>> {
        Some(&self.inner.database().cache.accounts)
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        self.inner.clear_into_state_snapshot()
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        self.inner.read_as_state_snapshot()
    }

    fn clear(&mut self) {
        self.inner.clear()
    }

    fn init_from_state_snapshot(&mut self, state_snapshot: StateSnapshot) {
        self.inner.init_from_state_snapshot(state_snapshot)
    }
}

// Implement MaybeForkedDatabase trait
impl MaybeForkedDatabase for OfflineForkedDatabase {
    fn maybe_reset(&mut self, url: Option<String>, block_number: BlockId) -> Result<(), String> {
        self.inner.maybe_reset(url, block_number)
    }

    fn maybe_flush_cache(&self) -> Result<(), String> {
        self.inner.maybe_flush_cache()
    }

    fn maybe_inner(&self) -> Result<&BlockchainDb, String> {
        self.inner.maybe_inner()
    }
}

// Implement Db trait
impl Db for OfflineForkedDatabase {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.inner.insert_account(address, account);
    }

    fn set_storage_at(&mut self, address: Address, slot: B256, val: B256) -> DatabaseResult<()> {
        self.inner.set_storage_at(address, slot, val)
    }

    fn insert_block_hash(&mut self, number: U256, hash: B256) {
        self.inner.insert_block_hash(number, hash);
    }

    fn dump_state(
        &self,
        at: BlockEnv,
        best_number: u64,
        blocks: Vec<SerializableBlock>,
        transactions: Vec<SerializableTransaction>,
        historical_states: Option<crate::eth::backend::db::SerializableHistoricalStates>,
    ) -> DatabaseResult<Option<SerializableState>> {
        self.inner.dump_state(at, best_number, blocks, transactions, historical_states)
    }

    fn snapshot_state(&mut self) -> U256 {
        self.inner.insert_state_snapshot()
    }

    fn revert_state(&mut self, id: U256, action: RevertStateSnapshotAction) -> bool {
        self.inner.revert_state_snapshot(id, action)
    }

    fn current_state(&self) -> StateDb {
        self.inner.current_state()
    }
}
