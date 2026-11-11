//! State forked off a remote client.

use crate::eth::backend::{
    db::{
        AccountInfo, AccountState, BlockEnv, BlockchainDb, Bytecode, CacheDB, DatabaseError,
        DatabaseRef, DatabaseResult, Db, DbAccount, EmptyDb, MaybeForkedDatabase,
        MaybeFullDatabase, RevertStateSnapshotAction, SerializableAccountRecord, SerializableBlock,
        SerializableHistoricalStates, SerializableState, SerializableTransaction, SharedBackend,
        StateDb, StateSnapshot, StateSnapshots, cache_block_hash,
    },
    mem::in_memory_db::{HistoricalStateCache, PersistentStateDb},
};
use alloy_network::Network;
use alloy_primitives::{Address, B256, Bytes, U256, map::AddressMap};
use alloy_rpc_types::BlockId;
use evm2::evm::BlockStateAccumulator;
use foundry_evm_core::evm::ForkCode;
use foundry_fork_db::cache::MemDb as RemoteState;
use parking_lot::Mutex;
use std::sync::{Arc, OnceLock};

/// Reads remote state through the RPC-backed fork cache.
#[derive(Clone, Debug)]
pub struct RemoteDb<N: Network> {
    backend: SharedBackend<N>,
    code: ForkCode,
}

impl<N: Network> RemoteDb<N> {
    /// Reads through `backend`, building bytecode into `code`.
    pub const fn new(backend: SharedBackend<N>, code: ForkCode) -> Self {
        Self { backend, code }
    }
}

impl<N: Network> DatabaseRef for RemoteDb<N> {
    fn basic_ref(&self, address: Address) -> DatabaseResult<Option<AccountInfo>> {
        let info = self.backend.account(address)?.map(|info| self.code.account(info));
        Ok(info.filter(|info| !info.is_empty()))
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> DatabaseResult<Bytecode> {
        self.code
            .code_by_hash(&self.backend.data(), code_hash)
            .ok_or(DatabaseError::MissingCode(code_hash))
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
fn remote_state_snapshot(remote: &RemoteState, code: &ForkCode) -> StateSnapshot {
    StateSnapshot {
        accounts: remote
            .accounts
            .read()
            .iter()
            .map(|(address, info)| (*address, code.account(info.clone())))
            .collect(),
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
    /// Tracks local changes for compact block states, see [`Self::with_compact_history`].
    history: Option<CompactHistory>,
}

/// The local changes recorded for compact block states.
#[derive(Debug, Default)]
struct CompactHistory(Mutex<HistoricalStateCache>);

impl Clone for CompactHistory {
    fn clone(&self) -> Self {
        Self(Mutex::new(self.0.lock().clone()))
    }
}

impl<N: Network> ForkedDatabase<N> {
    /// Creates a new instance of this DB.
    pub fn new(backend: SharedBackend<N>, db: BlockchainDb) -> Self {
        Self {
            cache_db: CacheDB::new(RemoteDb::new(backend.clone(), ForkCode::default())),
            backend,
            db,
            state_snapshots: Arc::new(Mutex::new(Default::default())),
            history: None,
        }
    }

    /// Makes the block states returned by [`Db::current_state`] compact.
    ///
    /// Each block state then shares everything unchanged with the previous one and reads state
    /// never changed locally from the pinned fork, instead of copying the local changes and the
    /// whole fork cache.
    pub fn with_compact_history(mut self) -> Self {
        self.history = Some(CompactHistory::default());
        self
    }

    /// Returns the history tracker if compact block states are enabled.
    fn history_mut(&mut self) -> Option<&mut HistoricalStateCache> {
        self.history.as_mut().map(|history| history.0.get_mut())
    }

    /// Discards the recorded history, so the next block state is rebuilt from the local state.
    fn invalidate_history(&mut self) {
        if let Some(history) = self.history_mut() {
            history.invalidate();
        }
    }

    pub const fn database(&self) -> &CacheDB<RemoteDb<N>> {
        &self.cache_db
    }

    /// Reset the fork to a fresh forked state, and optionally update the fork config.
    pub fn reset(
        &mut self,
        _urls: Vec<String>,
        block_number: impl Into<BlockId>,
    ) -> Result<(), String> {
        self.backend.set_pinned_block(block_number).map_err(|err| err.to_string())?;
        // wipe the storage retrieved from remote.
        self.inner().db().clear();
        // create a fresh `CacheDB`, effectively wiping modified state.
        self.cache_db = CacheDB::new(RemoteDb::new(self.backend.clone(), ForkCode::default()));
        self.invalidate_history();
        trace!(target: "backend::forkdb", "Cleared database");
        Ok(())
    }

    /// Flushes the cache to disk if configured.
    pub fn flush_cache(&self) {
        self.db.cache().flush()
    }

    /// Returns the database that holds the remote state.
    pub const fn inner(&self) -> &BlockchainDb {
        &self.db
    }

    /// Returns a read-only view of the current state.
    pub fn create_state_snapshot(&self) -> ForkDbStateSnapshot<N> {
        ForkDbStateSnapshot {
            local: self.cache_db.clone(),
            state_snapshot: remote_state_snapshot(self.db.db(), &self.cache_db.db.code),
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

    /// Removes the snapshot from the tracked snapshot and sets it as the current state.
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
        self.invalidate_history();
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
        if let Some(history) = self.history_mut() {
            history.record_account(address);
        }
        self.cache_db.insert_account_info(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: B256, val: B256) -> DatabaseResult<()> {
        if let Some(history) = self.history_mut() {
            history.record_storage(address, slot.into());
        }
        // this ensures the account is loaded first
        self.cache_db.insert_account_storage(address, slot.into(), val.into())
    }

    fn insert_block_hash(&mut self, number: U256, hash: B256) {
        if let Some(history) = self.history_mut() {
            history.record_block_hashes();
        }
        cache_block_hash(&mut self.cache_db.cache.block_hashes, number, hash);
    }

    fn set_block_hashes(&mut self, block_hashes: Vec<(U256, B256)>) {
        if let Some(history) = self.history_mut() {
            history.record_block_hashes();
        }
        self.cache_db.cache.block_hashes = block_hashes.into_iter().collect();
    }

    fn commit(&mut self, changes: &BlockStateAccumulator) {
        if let Some(history) = self.history_mut() {
            history.record_changes(changes);
        }
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
                        extension: Bytes::copy_from_slice(&account.info.extension),
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
        if let Some(history) = &self.history {
            return StateDb::new(CompactForkState::new(
                history.0.lock().snapshot(&self.cache_db.cache),
                self.cache_db.db.clone(),
            ));
        }
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
        // Callers may change any account, which the history cannot record.
        self.invalidate_history();
        Some(&mut self.cache_db.cache.accounts)
    }

    fn maybe_full_db(&self) -> Option<AddressMap<DbAccount>> {
        None
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        let db = self.inner().db();
        let state_snapshot = remote_state_snapshot(db, &self.cache_db.db.code);
        db.clear();
        state_snapshot
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        remote_state_snapshot(self.inner().db(), &self.cache_db.db.code)
    }

    fn clear(&mut self) {
        self.flush_cache();
        self.clear_into_state_snapshot();
    }

    fn init_from_state_snapshot(&mut self, state_snapshot: StateSnapshot) {
        // The fork cache only holds unchanged remote state, so restored state becomes local.
        self.inner().db().clear();
        self.cache_db.init_from_state_snapshot(state_snapshot);
        self.invalidate_history();
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

/// A compact, read-only block state of a [`ForkedDatabase`].
///
/// Local changes are kept in structurally shared maps, so consecutive block states share
/// everything unchanged. State never changed locally is read from the pinned fork, which does
/// not change while the block states exist: a fork reset clears them.
#[derive(Clone, Debug)]
pub struct CompactForkState<N: Network> {
    local: PersistentStateDb,
    remote: RemoteDb<N>,
    /// Lazily built local accounts, including the ones known not to exist.
    full: OnceLock<AddressMap<DbAccount>>,
}

impl<N: Network> CompactForkState<N> {
    const fn new(local: PersistentStateDb, remote: RemoteDb<N>) -> Self {
        Self { local, remote, full: OnceLock::new() }
    }

    /// Returns the local changes as a [`CacheDB`].
    fn local_cache_db(&self) -> CacheDB<EmptyDb> {
        let mut db = CacheDB::new(EmptyDb);
        db.cache = self.local.to_cache();
        db
    }
}

impl<N: Network> DatabaseRef for CompactForkState<N> {
    fn basic_ref(&self, address: Address) -> DatabaseResult<Option<AccountInfo>> {
        match self.local.cached_account(address) {
            Some(info) => Ok(info),
            None => self.remote.basic_ref(address),
        }
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> DatabaseResult<Bytecode> {
        match self.local.cached_code(code_hash) {
            Some(code) => Ok(code),
            None => self.remote.code_by_hash_ref(code_hash),
        }
    }

    fn storage_ref(&self, address: Address, index: U256) -> DatabaseResult<U256> {
        match self.local.cached_storage(address, index) {
            Some(value) => Ok(value),
            None => self.remote.storage_ref(address, index),
        }
    }

    fn block_hash_ref(&self, number: u64) -> DatabaseResult<B256> {
        match self.local.cached_block_hash(number) {
            Some(hash) => Ok(hash),
            None => self.remote.block_hash_ref(number),
        }
    }
}

impl<N: Network> MaybeFullDatabase for CompactForkState<N> {
    fn maybe_as_full_db(&self) -> Option<&AddressMap<DbAccount>> {
        Some(self.full.get_or_init(|| self.local.to_cache().accounts))
    }

    fn maybe_full_db(&self) -> Option<AddressMap<DbAccount>> {
        None
    }

    fn is_persistent(&self) -> bool {
        true
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        let state_snapshot = self.read_as_state_snapshot();
        self.clear();
        state_snapshot
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        // Includes the account states, so a restored state keeps falling back to the fork.
        self.local_cache_db().read_as_state_snapshot()
    }

    fn clear(&mut self) {
        self.local = PersistentStateDb::default();
        self.full = OnceLock::new();
    }

    fn init_from_state_snapshot(&mut self, state_snapshot: StateSnapshot) {
        let mut db = CacheDB::new(EmptyDb);
        db.init_from_state_snapshot(state_snapshot);
        self.local = PersistentStateDb::from_cache(&db.cache);
        self.full = OnceLock::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eth::backend::db::BlockchainDbMeta;
    use alloy_network::AnyNetwork;
    use alloy_primitives::address;
    use alloy_provider::ProviderBuilder;
    use alloy_transport::mock::Asserter;
    use foundry_fork_db::cache::AccountInfo as RemoteAccountInfo;

    const ACCOUNT: Address = address!("0x0000000000000000000000000000000000000001");
    const SLOT: U256 = U256::from_limbs([1, 0, 0, 0]);
    const OTHER_SLOT: U256 = U256::from_limbs([2, 0, 0, 0]);

    /// Returns a fork whose remote state is cached, so reads never reach the mocked endpoint.
    fn fork(compact: bool) -> ForkedDatabase<AnyNetwork> {
        let provider =
            ProviderBuilder::<_, _, AnyNetwork>::default().connect_mocked_client(Asserter::new());
        let remote = BlockchainDb::new(
            BlockchainDbMeta::new(BlockEnv::default(), "http://localhost".to_string()),
            None,
        );
        remote
            .db()
            .accounts
            .write()
            .insert(ACCOUNT, RemoteAccountInfo { balance: U256::from(5), ..Default::default() });
        remote.db().storage.write().insert(
            ACCOUNT,
            [(SLOT, U256::from(11)), (OTHER_SLOT, U256::from(22))].into_iter().collect(),
        );
        let backend = SharedBackend::spawn_backend_thread(provider, remote.clone(), None);
        let db = ForkedDatabase::new(backend, remote);
        if compact { db.with_compact_history() } else { db }
    }

    fn balance(state: &StateDb) -> U256 {
        state.basic_ref(ACCOUNT).unwrap().unwrap().balance
    }

    #[test]
    fn compact_history_states_are_isolated_and_read_through_to_fork() {
        let mut db = fork(true);
        let first = db.current_state();
        assert!(first.is_persistent());

        db.set_storage_at(ACCOUNT, SLOT.into(), B256::with_last_byte(100)).unwrap();
        let second = db.current_state();
        db.set_balance(ACCOUNT, U256::from(7)).unwrap();
        let third = db.current_state();

        assert_eq!(balance(&first), U256::from(5));
        assert_eq!(first.storage_ref(ACCOUNT, SLOT).unwrap(), U256::from(11));
        assert_eq!(balance(&second), U256::from(5));
        assert_eq!(second.storage_ref(ACCOUNT, SLOT).unwrap(), U256::from(100));
        assert_eq!(second.storage_ref(ACCOUNT, OTHER_SLOT).unwrap(), U256::from(22));
        assert_eq!(balance(&third), U256::from(7));
        assert_eq!(third.storage_ref(ACCOUNT, SLOT).unwrap(), U256::from(100));
        assert_eq!(third.storage_ref(ACCOUNT, OTHER_SLOT).unwrap(), U256::from(22));
    }

    #[test]
    fn compact_history_matches_full_history() {
        let mut compact = fork(true);
        let mut full = fork(false);
        let mut states = Vec::new();
        for db in [&mut compact, &mut full] {
            db.set_storage_at(ACCOUNT, SLOT.into(), B256::with_last_byte(100)).unwrap();
            let before_revert = db.snapshot_state();
            let first = db.current_state();

            db.set_storage_at(ACCOUNT, SLOT.into(), B256::with_last_byte(200)).unwrap();
            db.set_balance(ACCOUNT, U256::from(9)).unwrap();
            let second = db.current_state();

            assert!(db.revert_state(before_revert, RevertStateSnapshotAction::RevertRemove));
            let reverted = db.current_state();

            // Direct account access bypasses change recording.
            db.maybe_as_full_db_mut().unwrap().get_mut(&ACCOUNT).unwrap().account_state =
                AccountState::NotExisting;
            let deleted = db.current_state();

            states.push([first, second, reverted, deleted]);
        }

        for (compact, full) in states[0].iter().zip(&states[1]) {
            assert_eq!(compact.basic_ref(ACCOUNT).unwrap(), full.basic_ref(ACCOUNT).unwrap());
            for slot in [SLOT, OTHER_SLOT] {
                assert_eq!(
                    compact.storage_ref(ACCOUNT, slot).unwrap(),
                    full.storage_ref(ACCOUNT, slot).unwrap()
                );
            }
        }
        let [_, second, reverted, deleted] = &states[0];
        assert_eq!(second.storage_ref(ACCOUNT, SLOT).unwrap(), U256::from(200));
        assert_eq!(reverted.storage_ref(ACCOUNT, SLOT).unwrap(), U256::from(100));
        assert_eq!(deleted.basic_ref(ACCOUNT).unwrap(), None);
    }

    #[test]
    fn compact_history_state_restores_with_fork_fallback() {
        let mut db = fork(true);
        db.set_storage_at(ACCOUNT, SLOT.into(), B256::with_last_byte(100)).unwrap();
        db.insert_block_hash(U256::from(3), B256::with_last_byte(3));
        let deleted = Address::with_last_byte(2);
        db.insert_account(deleted, AccountInfo::default().with_balance(U256::ONE));
        db.maybe_as_full_db_mut().unwrap().get_mut(&deleted).unwrap().account_state =
            AccountState::NotExisting;
        let state = db.current_state();

        let state_snapshot = state.read_as_state_snapshot();
        assert!(!state_snapshot.storage[&ACCOUNT].contains_key(&OTHER_SLOT));
        assert!(state.maybe_as_full_db().unwrap().contains_key(&deleted));

        let restored = db.historical_state(state_snapshot);
        assert_eq!(restored.storage_ref(ACCOUNT, SLOT).unwrap(), U256::from(100));
        assert_eq!(restored.storage_ref(ACCOUNT, OTHER_SLOT).unwrap(), U256::from(22));
        assert_eq!(restored.basic_ref(deleted).unwrap(), None);
        assert_eq!(restored.block_hash_ref(3).unwrap(), B256::with_last_byte(3));
    }
}
