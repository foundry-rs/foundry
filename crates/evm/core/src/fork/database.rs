//! A revm database that forks off a remote client

use crate::{
    backend::{RevertStateSnapshotAction, StateSnapshot},
    state_snapshot::StateSnapshots,
};
use alloy_primitives::{Address, B256, U256};
use alloy_rpc_types::BlockId;
use foundry_fork_db::{BlockchainDb, DatabaseError, SharedBackend};
use parking_lot::Mutex;
use revm::{
    db::{CacheDB, DatabaseRef},
    primitives::{Account, AccountInfo, Bytecode, HashMap as Map},
    Database, DatabaseCommit,
};
use std::sync::Arc;

/// a [revm::Database] that's forked off another client
///
/// The `backend` is used to retrieve (missing) data, which is then fetched from the remote
/// endpoint. The inner in-memory database holds this storage and will be used for write operations.
/// This database uses the `backend` for read and the `db` for write operations. But note the
/// `backend` will also write (missing) data to the `db` in the background
#[derive(Clone, Debug)]
pub struct ForkedDatabase {
    /// Responsible for fetching missing data.
    ///
    /// This is responsible for getting data.
    backend: SharedBackend,
    /// Cached Database layer, ensures that changes are not written to the database that
    /// exclusively stores the state of the remote client.
    ///
    /// This separates Read/Write operations
    ///   - reads from the `SharedBackend as DatabaseRef` writes to the internal cache storage.
    cache_db: CacheDB<SharedBackend>,
    /// Contains all the data already fetched.
    ///
    /// This exclusively stores the _unchanged_ remote client state.
    db: BlockchainDb,
    /// Holds the state snapshots of a blockchain.
    state_snapshots: Arc<Mutex<StateSnapshots<ForkDbStateSnapshot>>>,
}

impl ForkedDatabase {
    /// Creates a new instance of this DB
    pub fn new(backend: SharedBackend, db: BlockchainDb) -> Self {
        Self {
            cache_db: CacheDB::new(backend.clone()),
            backend,
            db,
            state_snapshots: Arc::new(Mutex::new(Default::default())),
        }
    }

    pub fn database(&self) -> &CacheDB<SharedBackend> {
        &self.cache_db
    }

    pub fn database_mut(&mut self) -> &mut CacheDB<SharedBackend> {
        &mut self.cache_db
    }

    pub fn state_snapshots(&self) -> &Arc<Mutex<StateSnapshots<ForkDbStateSnapshot>>> {
        &self.state_snapshots
    }

    /// Reset the fork to a fresh forked state, and optionally update the fork config
    pub fn reset(
        &mut self,
        _url: Option<String>,
        block_number: impl Into<BlockId>,
    ) -> Result<(), String> {
        self.backend.set_pinned_block(block_number).map_err(|err| err.to_string())?;

        // TODO need to find a way to update generic provider via url

        // wipe the storage retrieved from remote
        self.inner().db().clear();
        // create a fresh `CacheDB`, effectively wiping modified state
        self.cache_db = CacheDB::new(self.backend.clone());
        trace!(target: "backend::forkdb", "Cleared database");
        Ok(())
    }

    /// Flushes the cache to disk if configured
    pub fn flush_cache(&self) {
        self.db.cache().flush()
    }

    /// Returns the database that holds the remote state
    pub fn inner(&self) -> &BlockchainDb {
        &self.db
    }

    pub fn create_state_snapshot(&self) -> ForkDbStateSnapshot {
        let db = self.db.db();
        let state_snapshot = StateSnapshot {
            accounts: db.accounts.read().clone(),
            storage: db.storage.read().clone(),
            block_hashes: db.block_hashes.read().clone(),
        };
        ForkDbStateSnapshot { local: self.cache_db.clone(), state_snapshot }
    }

    pub fn insert_state_snapshot(&self) -> U256 {
        let state_snapshot = self.create_state_snapshot();
        let mut state_snapshots = self.state_snapshots().lock();
        let id = state_snapshots.insert(state_snapshot);
        trace!(target: "backend::forkdb", "Created new snapshot {}", id);
        id
    }

    /// Removes the snapshot from the tracked snapshot and sets it as the current state
    pub fn revert_state_snapshot(&mut self, id: U256, action: RevertStateSnapshotAction) -> bool {
        let state_snapshot = { self.state_snapshots().lock().remove_at(id) };
        if let Some(state_snapshot) = state_snapshot {
            if action.is_keep() {
                self.state_snapshots().lock().insert_at(state_snapshot.clone(), id);
            }
            let ForkDbStateSnapshot {
                local,
                state_snapshot: StateSnapshot { accounts, storage, block_hashes },
            } = state_snapshot;
            let db = self.inner().db();
            {
                let mut accounts_lock = db.accounts.write();
                accounts_lock.clear();
                accounts_lock.extend(accounts);
            }
            {
                let mut storage_lock = db.storage.write();
                storage_lock.clear();
                storage_lock.extend(storage);
            }
            {
                let mut block_hashes_lock = db.block_hashes.write();
                block_hashes_lock.clear();
                block_hashes_lock.extend(block_hashes);
            }

            self.cache_db = local;

            trace!(target: "backend::forkdb", "Reverted snapshot {}", id);
            true
        } else {
            warn!(target: "backend::forkdb", "No snapshot to revert for {}", id);
            false
        }
    }
}

impl Database for ForkedDatabase {
    type Error = DatabaseError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        // Note: this will always return Some, since the `SharedBackend` will always load the
        // account, this differs from `<CacheDB as Database>::basic`, See also
        // [MemDb::ensure_loaded](crate::backend::MemDb::ensure_loaded)
        Database::basic(&mut self.cache_db, address)
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        Database::code_by_hash(&mut self.cache_db, code_hash)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Database::storage(&mut self.cache_db, address, index)
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        Database::block_hash(&mut self.cache_db, number)
    }
}

impl DatabaseRef for ForkedDatabase {
    type Error = DatabaseError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.cache_db.basic_ref(address)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.cache_db.code_by_hash_ref(code_hash)
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        DatabaseRef::storage_ref(&self.cache_db, address, index)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.cache_db.block_hash_ref(number)
    }
}

impl DatabaseCommit for ForkedDatabase {
    fn commit(&mut self, changes: Map<Address, Account>) {
        self.database_mut().commit(changes)
    }
}

/// Represents a snapshot of the database
///
/// This mimics `revm::CacheDB`
#[derive(Clone, Debug)]
pub struct ForkDbStateSnapshot {
    pub local: CacheDB<SharedBackend>,
    pub state_snapshot: StateSnapshot,
}

impl ForkDbStateSnapshot {
    fn get_storage(&self, address: Address, index: U256) -> Option<U256> {
        self.local.accounts.get(&address).and_then(|account| account.storage.get(&index)).copied()
    }
}

// This `DatabaseRef` implementation works similar to `CacheDB` which prioritizes modified elements,
// and uses another db as fallback
// We prioritize stored changed accounts/storage
impl DatabaseRef for ForkDbStateSnapshot {
    type Error = DatabaseError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        match self.local.accounts.get(&address) {
            Some(account) => Ok(Some(account.info.clone())),
            None => {
                let mut acc = self.state_snapshot.accounts.get(&address).cloned();

                if acc.is_none() {
                    acc = self.local.basic_ref(address)?;
                }
                Ok(acc)
            }
        }
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.local.code_by_hash_ref(code_hash)
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        match self.local.accounts.get(&address) {
            Some(account) => match account.storage.get(&index) {
                Some(entry) => Ok(*entry),
                None => match self.get_storage(address, index) {
                    None => DatabaseRef::storage_ref(&self.local, address, index),
                    Some(storage) => Ok(storage),
                },
            },
            None => match self.get_storage(address, index) {
                None => DatabaseRef::storage_ref(&self.local, address, index),
                Some(storage) => Ok(storage),
            },
        }
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        match self.state_snapshot.block_hashes.get(&U256::from(number)).copied() {
            None => self.local.block_hash_ref(number),
            Some(block_hash) => Ok(block_hash),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::needless_return)]

    use super::*;
    use crate::backend::BlockchainDbMeta;
    use foundry_common::provider::get_http_provider;
    use std::collections::BTreeSet;

    /// Demonstrates that `Database::basic` for `ForkedDatabase` will always return the
    /// `AccountInfo`
    #[tokio::test(flavor = "multi_thread")]
    async fn fork_db_insert_basic_default() {
        let rpc = foundry_test_utils::rpc::next_http_rpc_endpoint();
        let provider = get_http_provider(rpc.clone());
        let meta = BlockchainDbMeta {
            cfg_env: Default::default(),
            block_env: Default::default(),
            hosts: BTreeSet::from([rpc]),
        };
        let db = BlockchainDb::new(meta, None);

        let backend = SharedBackend::spawn_backend(Arc::new(provider), db.clone(), None).await;

        let mut db = ForkedDatabase::new(backend, db);
        let address = Address::random();

        let info = Database::basic(&mut db, address).unwrap();
        assert!(info.is_some());
        let mut info = info.unwrap();
        info.balance = U256::from(500u64);

        // insert the modified account info
        db.database_mut().insert_account_info(address, info.clone());

        let loaded = Database::basic(&mut db, address).unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap(), info);
    }
}
