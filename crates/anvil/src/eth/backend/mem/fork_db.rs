//! State forked off a remote client.

use crate::eth::backend::db::{
    AccountInfo, AccountState, BlockEnv, BlockchainDb, Bytecode, CacheDB, DatabaseError,
    DatabaseRef, DatabaseResult, Db, DbAccount, MaybeForkedDatabase, MaybeFullDatabase,
    RevertStateSnapshotAction, SerializableAccountRecord, SerializableBlock,
    SerializableHistoricalStates, SerializableState, SerializableTransaction, SharedBackend,
    StateDb, StateSnapshot, StateSnapshots, cache_block_hash,
};
use alloy_network::Network;
use alloy_primitives::{Address, B256, U256, map::AddressMap};
use alloy_rpc_types::BlockId;
use evm2::evm::BlockStateAccumulator;
use foundry_fork_db::cache::MemDb as RemoteState;
use parking_lot::Mutex;
use std::sync::Arc;

/// Reads remote state through the RPC-backed fork cache.
#[derive(Clone, Debug)]
pub struct RemoteDb<N: Network> {
    backend: SharedBackend<N>,
}

impl<N: Network> RemoteDb<N> {
    /// Reads through `backend`.
    pub const fn new(backend: SharedBackend<N>) -> Self {
        Self { backend }
    }
}

impl<N: Network> DatabaseRef for RemoteDb<N> {
    fn basic_ref(&self, address: Address) -> DatabaseResult<Option<AccountInfo>> {
        self.backend.account(address).map(|info| info.filter(|info| !info.is_empty()))
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> DatabaseResult<Bytecode> {
        Err(DatabaseError::MissingCode(code_hash))
    }

    fn storage_ref(&self, address: Address, index: U256) -> DatabaseResult<U256> {
        self.backend.storage_at(address, index)
    }

    fn block_hash_ref(&self, number: u64) -> DatabaseResult<B256> {
        self.backend.block_hash(number)
    }
}

impl<N: Network> MaybeFullDatabase for RemoteDb<N> {
    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        StateSnapshot::default()
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        StateSnapshot::default()
    }

    fn clear(&mut self) {}

    fn init_from_state_snapshot(&mut self, _state_snapshot: StateSnapshot) {}
}

/// Reads the accounts, storage and block hashes held by the fork cache.
fn remote_state_snapshot(remote: &RemoteState) -> StateSnapshot {
    StateSnapshot {
        accounts: remote.accounts.read().clone(),
        storage: remote
            .storage
            .read()
            .iter()
            .map(|(address, storage)| {
                (*address, storage.iter().map(|(slot, value)| (*slot, *value)).collect())
            })
            .collect(),
        block_hashes: remote.block_hashes.read().clone(),
        account_states: None,
    }
}

/// A local snapshot of a [`ForkedDatabase`]: its local changes and the fork cache's contents.
#[derive(Clone, Debug)]
struct ForkDbLocalSnapshot<N: Network> {
    local: CacheDB<RemoteDb<N>>,
    remote: RemoteState,
}

/// State forked off another client.
///
/// Missing state is fetched from the remote endpoint through the fork cache (`db`); changes are
/// kept in a local [`CacheDB`] so the fork cache only ever holds unchanged remote state.
#[derive(Clone, Debug)]
pub struct ForkedDatabase<N: Network> {
    /// Responsible for fetching missing data.
    backend: SharedBackend<N>,
    /// Local changes over the remote state.
    cache_db: CacheDB<RemoteDb<N>>,
    /// Contains all the data already fetched; exclusively the unchanged remote state.
    db: BlockchainDb,
    /// Holds the state snapshots of a blockchain.
    state_snapshots: Arc<Mutex<StateSnapshots<ForkDbLocalSnapshot<N>>>>,
}

impl<N: Network> ForkedDatabase<N> {
    /// Creates a new instance of this DB
    pub fn new(backend: SharedBackend<N>, db: BlockchainDb) -> Self {
        Self {
            cache_db: CacheDB::new(RemoteDb::new(backend.clone())),
            backend,
            db,
            state_snapshots: Arc::new(Mutex::new(Default::default())),
        }
    }

    pub const fn database(&self) -> &CacheDB<RemoteDb<N>> {
        &self.cache_db
    }

    /// Reset the fork to a fresh forked state, and optionally update the fork config
    pub fn reset(
        &mut self,
        _urls: Vec<String>,
        block_number: impl Into<BlockId>,
    ) -> Result<(), String> {
        self.backend.set_pinned_block(block_number).map_err(|err| err.to_string())?;
        // wipe the storage retrieved from remote
        self.inner().db().clear();
        // create a fresh `CacheDB`, effectively wiping modified state
        self.cache_db = CacheDB::new(RemoteDb::new(self.backend.clone()));
        trace!(target: "backend::forkdb", "Cleared database");
        Ok(())
    }

    /// Flushes the cache to disk if configured
    pub fn flush_cache(&self) {
        self.db.cache().flush()
    }

    /// Returns the database that holds the remote state
    pub const fn inner(&self) -> &BlockchainDb {
        &self.db
    }

    /// Returns a read-only view of the current state.
    pub fn create_state_snapshot(&self) -> ForkDbStateSnapshot<N> {
        ForkDbStateSnapshot {
            local: self.cache_db.clone(),
            state_snapshot: remote_state_snapshot(self.db.db()),
        }
    }

    /// Creates a new snapshot and returns its ID.
    pub fn insert_state_snapshot(&self) -> U256 {
        let state_snapshot = ForkDbLocalSnapshot {
            local: self.cache_db.clone(),
            remote: RemoteState::clone(self.db.db()),
        };
        let id = self.state_snapshots.lock().insert(state_snapshot);
        trace!(target: "backend::forkdb", "Created new snapshot {}", id);
        id
    }

    /// Removes the snapshot from the tracked snapshot and sets it as the current state
    pub fn revert_state_snapshot(&mut self, id: U256, action: RevertStateSnapshotAction) -> bool {
        let state_snapshot = self.state_snapshots.lock().remove(id);
        let Some(state_snapshot) = state_snapshot else {
            warn!(target: "backend::forkdb", "No snapshot to revert for {}", id);
            return false;
        };
        if action.is_keep() {
            self.state_snapshots.lock().insert_at(state_snapshot.clone(), id);
        }
        let ForkDbLocalSnapshot { local, remote } = state_snapshot;
        let db = self.inner().db();
        *db.accounts.write() = remote.accounts.into_inner();
        *db.storage.write() = remote.storage.into_inner();
        *db.block_hashes.write() = remote.block_hashes.into_inner();
        self.cache_db = local;
        trace!(target: "backend::forkdb", "Reverted snapshot {}", id);
        true
    }
}

impl<N: Network> DatabaseRef for ForkedDatabase<N> {
    fn basic_ref(&self, address: Address) -> DatabaseResult<Option<AccountInfo>> {
        self.cache_db.basic_ref(address)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> DatabaseResult<Bytecode> {
        self.cache_db.code_by_hash_ref(code_hash)
    }

    fn storage_ref(&self, address: Address, index: U256) -> DatabaseResult<U256> {
        self.cache_db.storage_ref(address, index)
    }

    fn block_hash_ref(&self, number: u64) -> DatabaseResult<B256> {
        self.cache_db.block_hash_ref(number)
    }
}

impl<N: Network> Db for ForkedDatabase<N> {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.cache_db.insert_account_info(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: B256, val: B256) -> DatabaseResult<()> {
        // this ensures the account is loaded first
        self.cache_db.insert_account_storage(address, slot.into(), val.into())
    }

    fn insert_block_hash(&mut self, number: U256, hash: B256) {
        cache_block_hash(&mut self.cache_db.cache.block_hashes, number, hash);
    }

    fn set_block_hashes(&mut self, block_hashes: Vec<(U256, B256)>) {
        self.cache_db.cache.block_hashes = block_hashes.into_iter().collect();
    }

    fn commit(&mut self, changes: &BlockStateAccumulator) {
        self.cache_db.commit(changes)
    }

    fn dump_state(
        &self,
        at: BlockEnv,
        best_number: u64,
        blocks: Vec<SerializableBlock>,
        transactions: Vec<SerializableTransaction>,
        historical_states: Option<SerializableHistoricalStates>,
    ) -> DatabaseResult<Option<SerializableState>> {
        let accounts = self
            .cache_db
            .cache
            .accounts
            .iter()
            .filter(|(_, account)| account.account_state != AccountState::NotExisting)
            .map(|(address, account)| -> DatabaseResult<_> {
                let code = match &account.info.code {
                    Some(code) => code.clone(),
                    None => self.cache_db.code_by_hash_ref(account.info.code_hash)?,
                };
                Ok((
                    *address,
                    SerializableAccountRecord {
                        nonce: account.info.nonce,
                        balance: account.info.balance,
                        code: code.original_bytes(),
                        storage: account
                            .storage
                            .iter()
                            .map(|(k, v)| ((*k).into(), (*v).into()))
                            .collect(),
                    },
                ))
            })
            .collect::<Result<_, _>>()?;
        Ok(Some(SerializableState {
            block: Some(at),
            accounts,
            best_block_number: Some(best_number),
            blocks,
            transactions,
            #[cfg(feature = "monad")]
            monad_block_participants: Default::default(),
            #[cfg(feature = "monad")]
            monad_block_replay_profiles: Default::default(),
            historical_states,
        }))
    }

    fn snapshot_state(&mut self) -> U256 {
        self.insert_state_snapshot()
    }

    fn revert_state(&mut self, id: U256, action: RevertStateSnapshotAction) -> bool {
        self.revert_state_snapshot(id, action)
    }

    fn delete_state_snapshot(&mut self, id: U256) -> bool {
        self.state_snapshots.lock().remove_at(id).is_some()
    }

    fn current_state(&self) -> StateDb {
        StateDb::new(self.create_state_snapshot())
    }

    fn historical_state(&self, state_snapshot: StateSnapshot) -> StateDb {
        // Fall back to the fork for anything the snapshot did not capture. Snapshots without
        // account states cannot tell cleared storage from uncached storage, so they keep the
        // previous standalone restore.
        let mut state = if state_snapshot.account_states.is_some() {
            StateDb::new(CacheDB::new(self.database().db.clone()))
        } else {
            StateDb::new(super::in_memory_db::MemDb::default())
        };
        state.init_from_state_snapshot(state_snapshot);
        state
    }
}

impl<N: Network> MaybeFullDatabase for ForkedDatabase<N> {
    fn maybe_as_full_db(&self) -> Option<&AddressMap<DbAccount>> {
        Some(&self.cache_db.cache.accounts)
    }

    fn maybe_as_full_db_mut(&mut self) -> Option<&mut AddressMap<DbAccount>> {
        Some(&mut self.cache_db.cache.accounts)
    }

    fn maybe_full_db(&self) -> Option<AddressMap<DbAccount>> {
        None
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        let db = self.inner().db();
        let state_snapshot = remote_state_snapshot(db);
        db.clear();
        state_snapshot
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        remote_state_snapshot(self.inner().db())
    }

    fn clear(&mut self) {
        self.flush_cache();
        self.clear_into_state_snapshot();
    }

    fn init_from_state_snapshot(&mut self, state_snapshot: StateSnapshot) {
        // The fork cache only holds unchanged remote state, so restored state becomes local.
        self.inner().db().clear();
        self.cache_db.init_from_state_snapshot(state_snapshot);
    }
}

impl<N: Network> MaybeForkedDatabase for ForkedDatabase<N> {
    fn maybe_reset(&mut self, urls: Vec<String>, block_number: BlockId) -> Result<(), String> {
        self.reset(urls, block_number)
    }

    fn maybe_flush_cache(&self) -> Result<(), String> {
        self.flush_cache();
        Ok(())
    }

    fn maybe_inner(&self) -> Result<&BlockchainDb, String> {
        Ok(self.inner())
    }
}

/// A read-only view of a [`ForkedDatabase`] at a point in time.
///
/// Local changes take precedence over the remote state captured with the snapshot, which takes
/// precedence over the live fork cache.
#[derive(Clone, Debug)]
pub struct ForkDbStateSnapshot<N: Network> {
    pub local: CacheDB<RemoteDb<N>>,
    pub state_snapshot: StateSnapshot,
}

impl<N: Network> ForkDbStateSnapshot<N> {
    /// Lookup storage in `state_snapshot`, then fall back to the backend (remote RPC).
    fn storage_from_snapshot_or_backend(
        &self,
        address: Address,
        index: U256,
    ) -> DatabaseResult<U256> {
        if let Some(val) = self.state_snapshot.storage.get(&address).and_then(|s| s.get(&index)) {
            return Ok(*val);
        }
        self.local.storage_ref(address, index)
    }
}

impl<N: Network> DatabaseRef for ForkDbStateSnapshot<N> {
    fn basic_ref(&self, address: Address) -> DatabaseResult<Option<AccountInfo>> {
        match self.local.cache.accounts.get(&address) {
            Some(account) => Ok(account.info()),
            None => match self.state_snapshot.accounts.get(&address).cloned() {
                Some(info) => Ok(Some(info)),
                None => self.local.basic_ref(address),
            },
        }
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> DatabaseResult<Bytecode> {
        self.local.code_by_hash_ref(code_hash)
    }

    fn storage_ref(&self, address: Address, index: U256) -> DatabaseResult<U256> {
        match self.local.cache.accounts.get(&address) {
            Some(account) => match account.storage.get(&index) {
                Some(entry) => Ok(*entry),
                None if account.account_state.is_storage_cleared() => Ok(U256::ZERO),
                None => self.storage_from_snapshot_or_backend(address, index),
            },
            None => self.storage_from_snapshot_or_backend(address, index),
        }
    }

    fn block_hash_ref(&self, number: u64) -> DatabaseResult<B256> {
        match self.state_snapshot.block_hashes.get(&U256::from(number)).copied() {
            None => self.local.block_hash_ref(number),
            Some(block_hash) => Ok(block_hash),
        }
    }
}

impl<N: Network> MaybeFullDatabase for ForkDbStateSnapshot<N> {
    fn maybe_as_full_db(&self) -> Option<&AddressMap<DbAccount>> {
        Some(&self.local.cache.accounts)
    }

    fn maybe_full_db(&self) -> Option<AddressMap<DbAccount>> {
        None
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        let mut state_snapshot = std::mem::take(&mut self.state_snapshot);
        let local_state_snapshot = self.local.clear_into_state_snapshot();
        state_snapshot.accounts.extend(local_state_snapshot.accounts);
        state_snapshot.storage.extend(local_state_snapshot.storage);
        state_snapshot.block_hashes.extend(local_state_snapshot.block_hashes);
        if let Some(account_states) = local_state_snapshot.account_states {
            state_snapshot.account_states.get_or_insert_default().extend(account_states);
        }
        state_snapshot
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        let mut state_snapshot = self.state_snapshot.clone();
        let local_state_snapshot = self.local.read_as_state_snapshot();
        state_snapshot.accounts.extend(local_state_snapshot.accounts);
        state_snapshot.storage.extend(local_state_snapshot.storage);
        state_snapshot.block_hashes.extend(local_state_snapshot.block_hashes);
        if let Some(account_states) = local_state_snapshot.account_states {
            state_snapshot.account_states.get_or_insert_default().extend(account_states);
        }
        state_snapshot
    }

    fn clear(&mut self) {
        std::mem::take(&mut self.state_snapshot);
        self.local.clear()
    }

    fn init_from_state_snapshot(&mut self, state_snapshot: StateSnapshot) {
        self.state_snapshot = state_snapshot;
    }
}
