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
use std::sync::OnceLock;

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

/// State forked off another client.
///
/// Missing state is fetched from the remote endpoint through the fork cache (`db`); changes are
/// kept in a local [`CacheDB`] so the fork cache only ever holds unchanged remote state.
#[derive(Debug)]
pub struct ForkedDatabase<N: Network> {
    /// Local changes over the remote state.
    cache_db: CacheDB<RemoteDb<N>>,
    /// Contains all the data already fetched; exclusively the unchanged remote state.
    db: BlockchainDb,
    /// Holds the local changes at each state snapshot. The fork cache only holds unchanged remote
    /// state, which retained block states read and dump, so reverting keeps it.
    state_snapshots: Mutex<StateSnapshots<CacheDB<RemoteDb<N>>>>,
    /// Records local changes, so each block state shares everything unchanged with the previous.
    history: Mutex<HistoricalStateCache>,
}

impl<N: Network> ForkedDatabase<N> {
    /// Creates a new instance of this DB, recording local changes for block states only if
    /// `track_history` is set, as in [`StateRootDb::new`](super::in_memory_db::StateRootDb::new).
    pub fn new(backend: SharedBackend<N>, db: BlockchainDb, track_history: bool) -> Self {
        Self {
            cache_db: CacheDB::new(RemoteDb::new(backend, ForkCode::default())),
            db,
            state_snapshots: Default::default(),
            history: Mutex::new(HistoricalStateCache::new(track_history)),
        }
    }

    /// Discards the recorded history, so the next block state is rebuilt from the local state.
    fn invalidate_history(&mut self) {
        self.history.get_mut().invalidate();
    }

    pub const fn database(&self) -> &CacheDB<RemoteDb<N>> {
        &self.cache_db
    }

    /// Flushes the cache to disk if configured.
    pub fn flush_cache(&self) {
        self.db.cache().flush()
    }

    /// Returns the database that holds the remote state.
    pub const fn inner(&self) -> &BlockchainDb {
        &self.db
    }

    /// Creates a new snapshot and returns its ID.
    pub fn insert_state_snapshot(&self) -> U256 {
        let id = self.state_snapshots.lock().insert(self.cache_db.clone());
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
        self.cache_db = state_snapshot;
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
        self.history.get_mut().record_account(address);
        self.cache_db.insert_account_info(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: B256, val: B256) -> DatabaseResult<()> {
        self.history.get_mut().record_storage(address, slot.into());
        // this ensures the account is loaded first
        self.cache_db.insert_account_storage(address, slot.into(), val.into())
    }

    fn insert_block_hash(&mut self, number: U256, hash: B256) {
        self.history.get_mut().record_block_hash(number, hash, false);
        cache_block_hash(&mut self.cache_db.cache.block_hashes, number, hash);
    }

    fn set_block_hashes(&mut self, block_hashes: Vec<(U256, B256)>) {
        self.invalidate_history();
        self.cache_db.cache.block_hashes = block_hashes.into_iter().collect();
    }

    fn commit(&mut self, changes: &BlockStateAccumulator) {
        self.history.get_mut().record_changes(changes);
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
        StateDb::new(ForkDbStateSnapshot::new(
            self.history.lock().snapshot(&self.cache_db.cache),
            self.cache_db.db.clone(),
        ))
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
        // Keeps the fork cache: it only holds unchanged remote state, which retained block states
        // read and dump.
    }

    fn init_from_state_snapshot(&mut self, state_snapshot: StateSnapshot) {
        // The fork cache only holds unchanged remote state, so restored state becomes local.
        self.inner().db().clear();
        self.cache_db.init_from_state_snapshot(state_snapshot);
        self.invalidate_history();
    }
}

impl<N: Network> MaybeForkedDatabase for ForkedDatabase<N> {
    // Retained block states read through the pinned backend, so a fork reset replaces the
    // database instead of re-pinning it.
    fn maybe_reset(&mut self, _urls: Vec<String>, _block_number: BlockId) -> Result<(), String> {
        Err("not supported".to_string())
    }

    fn maybe_flush_cache(&self) -> Result<(), String> {
        self.flush_cache();
        Ok(())
    }

    fn maybe_inner(&self) -> Result<&BlockchainDb, String> {
        Ok(self.inner())
    }
}

/// A read-only block state of a [`ForkedDatabase`].
///
/// Local changes are kept in structurally shared maps, so consecutive block states share
/// everything unchanged. State never changed locally is read from the pinned fork, which does
/// not change while the block states exist: the backend drops them when it resets the fork.
#[derive(Clone, Debug)]
pub struct ForkDbStateSnapshot<N: Network> {
    local: PersistentStateDb,
    remote: RemoteDb<N>,
    /// Lazily built local accounts, including the ones known not to exist.
    full: OnceLock<AddressMap<DbAccount>>,
}

impl<N: Network> ForkDbStateSnapshot<N> {
    const fn new(local: PersistentStateDb, remote: RemoteDb<N>) -> Self {
        Self { local, remote, full: OnceLock::new() }
    }
}

impl<N: Network> DatabaseRef for ForkDbStateSnapshot<N> {
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

impl<N: Network> MaybeFullDatabase for ForkDbStateSnapshot<N> {
    fn maybe_as_full_db(&self) -> Option<&AddressMap<DbAccount>> {
        Some(self.full.get_or_init(|| self.local.full_db(true)))
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
        // Includes the state fetched from the fork, so the state restores without it, and the
        // account states, so a restored state falls back to the fork for the rest.
        let mut state_snapshot =
            remote_state_snapshot(&self.remote.backend.data(), &self.remote.code);
        let mut local = CacheDB::new(EmptyDb);
        local.cache = self.local.to_cache();
        let local = local.read_as_state_snapshot();
        let account_states = local.account_states.unwrap_or_default();
        for (address, storage) in local.storage {
            if account_states.get(&address).is_some_and(AccountState::is_storage_cleared) {
                state_snapshot.storage.insert(address, storage);
            } else {
                state_snapshot.storage.entry(address).or_default().extend(storage);
            }
        }
        state_snapshot.accounts.extend(local.accounts);
        state_snapshot.block_hashes.extend(local.block_hashes);
        state_snapshot.account_states = Some(account_states);
        state_snapshot
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
    use crate::eth::backend::{db::BlockchainDbMeta, mem::in_memory_db::MemDb};
    use alloy_network::AnyNetwork;
    use alloy_primitives::address;
    use alloy_provider::ProviderBuilder;
    use alloy_transport::mock::Asserter;
    use foundry_fork_db::cache::AccountInfo as RemoteAccountInfo;

    const ACCOUNT: Address = address!("0x0000000000000000000000000000000000000001");
    const SLOT: U256 = U256::from_limbs([1, 0, 0, 0]);
    const OTHER_SLOT: U256 = U256::from_limbs([2, 0, 0, 0]);
    const FETCHED_SLOT: U256 = U256::from_limbs([3, 0, 0, 0]);
    const DELETED: Address = address!("0x0000000000000000000000000000000000000002");

    /// Returns a fork whose remote state is cached, so reads never reach the mocked endpoint.
    fn fork() -> ForkedDatabase<AnyNetwork> {
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
        remote.db().storage.write().extend([
            (ACCOUNT, [(SLOT, U256::from(11)), (OTHER_SLOT, U256::from(22))].into_iter().collect()),
            (DELETED, [(SLOT, U256::from(33))].into_iter().collect()),
        ]);
        remote.db().block_hashes.write().extend(
            [1, 300].map(|number| (U256::from(number), B256::with_last_byte(number as u8))),
        );
        let backend = SharedBackend::spawn_backend_thread(provider, remote.clone(), None);
        ForkedDatabase::new(backend, remote, true)
    }

    #[test]
    fn fork_block_states_match_live_state() {
        let mut db = fork();
        // Each block state must read like a copy of the live local state taken at the same time.
        let mut states = Vec::new();
        let mut take = |db: &ForkedDatabase<AnyNetwork>| {
            states.push((db.current_state(), db.database().clone()));
        };
        db.set_storage_at(ACCOUNT, SLOT.into(), B256::with_last_byte(100)).unwrap();
        db.insert_block_hash(U256::ONE, B256::with_last_byte(0xaa));
        let before_revert = db.snapshot_state();
        take(&db);

        db.set_storage_at(ACCOUNT, SLOT.into(), B256::with_last_byte(200)).unwrap();
        // Evicts the local hash of block 1, so it reads from the fork again.
        db.insert_block_hash(U256::from(300), B256::with_last_byte(0xbb));
        take(&db);

        // Changes only the account info.
        db.set_balance(ACCOUNT, U256::from(9)).unwrap();
        take(&db);

        assert!(db.revert_state(before_revert, RevertStateSnapshotAction::RevertRemove));
        take(&db);

        // Replaces the cached block hashes, as loading a state does.
        db.set_block_hashes(vec![(U256::ONE, B256::with_last_byte(0xcc))]);
        take(&db);

        // Direct account access bypasses change recording.
        db.maybe_as_full_db_mut().unwrap().get_mut(&ACCOUNT).unwrap().account_state =
            AccountState::NotExisting;
        take(&db);

        for (state, live) in &states {
            assert!(state.is_persistent());
            assert_eq!(state.basic_ref(ACCOUNT).unwrap(), live.basic_ref(ACCOUNT).unwrap());
            for number in [1, 300] {
                assert_eq!(
                    state.block_hash_ref(number).unwrap(),
                    live.block_hash_ref(number).unwrap()
                );
            }
            for slot in [SLOT, OTHER_SLOT] {
                assert_eq!(
                    state.storage_ref(ACCOUNT, slot).unwrap(),
                    live.storage_ref(ACCOUNT, slot).unwrap()
                );
            }
        }
        let [_, (second, _), _, (reverted, _), _, (deleted, _)] = &states[..] else {
            unreachable!()
        };
        assert_eq!(second.storage_ref(ACCOUNT, SLOT).unwrap(), U256::from(200));
        assert_eq!(reverted.storage_ref(ACCOUNT, SLOT).unwrap(), U256::from(100));
        assert_eq!(deleted.basic_ref(ACCOUNT).unwrap(), None);
    }

    #[test]
    fn block_state_restores_with_and_without_fork() {
        let mut db = fork();
        let snapshot = db.snapshot_state();
        // Fetched from the fork after the snapshot.
        db.inner()
            .db()
            .storage
            .write()
            .get_mut(&ACCOUNT)
            .unwrap()
            .insert(FETCHED_SLOT, U256::from(44));
        db.set_storage_at(ACCOUNT, SLOT.into(), B256::with_last_byte(100)).unwrap();
        db.insert_block_hash(U256::from(3), B256::with_last_byte(3));
        db.insert_account(DELETED, AccountInfo::default().with_balance(U256::ONE));
        db.maybe_as_full_db_mut().unwrap().get_mut(&DELETED).unwrap().account_state =
            AccountState::NotExisting;
        let state = db.current_state();
        assert!(state.maybe_as_full_db().unwrap().contains_key(&DELETED));
        // Rollback clears the database and `evm_revert` reverts it, which must both keep the fork
        // state block states fetched.
        db.clear();
        assert!(db.revert_state(snapshot, RevertStateSnapshotAction::RevertRemove));

        // A dumped state restores on the fork and, with the state fetched from it, standalone.
        let standalone = MemDb::default().historical_state(state.read_as_state_snapshot());
        let forked = db.historical_state(state.read_as_state_snapshot());
        for restored in [standalone, forked] {
            assert_eq!(restored.storage_ref(ACCOUNT, SLOT).unwrap(), U256::from(100));
            assert_eq!(restored.storage_ref(ACCOUNT, OTHER_SLOT).unwrap(), U256::from(22));
            assert_eq!(restored.storage_ref(ACCOUNT, FETCHED_SLOT).unwrap(), U256::from(44));
            assert_eq!(restored.basic_ref(DELETED).unwrap(), None);
            assert_eq!(restored.storage_ref(DELETED, SLOT).unwrap(), U256::ZERO);
            assert_eq!(restored.block_hash_ref(3).unwrap(), B256::with_last_byte(3));
        }
    }
}
