//! A revm database that forks off a remote client

use crate::{
    executor::{
        fork::{BlockchainDb, SharedBackend},
        snapshot::Snapshots,
    },
    revm::db::CacheDB,
};
use ethers::prelude::{Address, H256, U256};
use hashbrown::HashMap as Map;
use parking_lot::Mutex;
use revm::{db::DatabaseRef, Account, AccountInfo, Bytecode, Database, DatabaseCommit};
use std::{collections::BTreeMap, sync::Arc};
use tracing::{trace, warn};

/// a [revm::Database] that's forked off another client
///
/// The `backend` is used to retrieve (missing) data, which is then fetched from the remote
/// endpoint. The inner in-memory database holds this storage and will be used for write operations.
/// This database uses the `backend` for read and the `db` for write operations. But note the
/// `backend` will also write (missing) data to the `db` in the background
#[derive(Debug, Clone)]
pub struct ForkedDatabase {
    /// responsible for fetching missing data
    ///
    /// This is responsible for getting data
    backend: SharedBackend,
    /// Cached Database layer, ensures that changes are not written to the database that
    /// exclusively stores the state of the remote client.
    ///
    /// This separates Read/Write operations
    ///   - reads from the `SharedBackend as DatabaseRef` writes to the internal cache storage
    cache_db: CacheDB<SharedBackend>,
    /// Contains all the data already fetched
    ///
    /// This exclusively stores the _unchanged_ remote client state
    db: BlockchainDb,
    /// holds the snapshot state of a blockchain
    snapshots: Arc<Mutex<Snapshots<ForkDbSnapshot>>>,
}

impl ForkedDatabase {
    /// Creates a new instance of this DB
    pub fn new(backend: SharedBackend, db: BlockchainDb) -> Self {
        Self {
            cache_db: CacheDB::new(backend.clone()),
            backend,
            db,
            snapshots: Arc::new(Mutex::new(Default::default())),
        }
    }

    pub fn database(&self) -> &CacheDB<SharedBackend> {
        &self.cache_db
    }

    pub fn database_mut(&mut self) -> &mut CacheDB<SharedBackend> {
        &mut self.cache_db
    }

    pub fn snapshots(&self) -> &Arc<Mutex<Snapshots<ForkDbSnapshot>>> {
        &self.snapshots
    }

    /// Reset the fork to a fresh forked state, and optionally update the fork config
    pub fn reset(&mut self, _url: Option<String>, block_number: Option<u64>) -> Result<(), String> {
        if let Some(block_number) = block_number {
            self.backend.set_pinned_block(block_number).map_err(|err| err.to_string())?;
        }

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

    pub fn create_snapshot(&self) -> ForkDbSnapshot {
        let db = self.db.db();
        ForkDbSnapshot {
            local: self.cache_db.clone(),
            accounts: db.accounts.read().clone(),
            storage: db.storage.read().clone(),
            block_hashes: db.block_hashes.read().clone(),
        }
    }

    pub fn insert_snapshot(&self) -> U256 {
        let snapshot = self.create_snapshot();
        let mut snapshots = self.snapshots().lock();
        let id = snapshots.insert(snapshot);
        trace!(target: "backend::forkdb", "Created new snapshot {}", id);
        id
    }

    pub fn revert_snapshot(&mut self, id: U256) -> bool {
        let snapshot = { self.snapshots().lock().remove(id) };
        if let Some(snapshot) = snapshot {
            let ForkDbSnapshot { accounts, storage, block_hashes, local } = snapshot;
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
    fn basic(&mut self, address: Address) -> AccountInfo {
        Database::basic(&mut self.cache_db, address)
    }

    fn code_by_hash(&mut self, code_hash: H256) -> Bytecode {
        Database::code_by_hash(&mut self.cache_db, code_hash)
    }

    fn storage(&mut self, address: Address, index: U256) -> U256 {
        Database::storage(&mut self.cache_db, address, index)
    }

    fn block_hash(&mut self, number: U256) -> H256 {
        Database::block_hash(&mut self.cache_db, number)
    }
}

impl DatabaseRef for ForkedDatabase {
    fn basic(&self, address: Address) -> AccountInfo {
        self.cache_db.basic(address)
    }

    fn code_by_hash(&self, code_hash: H256) -> Bytecode {
        self.cache_db.code_by_hash(code_hash)
    }

    fn storage(&self, address: Address, index: U256) -> U256 {
        DatabaseRef::storage(&self.cache_db, address, index)
    }

    fn block_hash(&self, number: U256) -> H256 {
        self.cache_db.block_hash(number)
    }
}

impl DatabaseCommit for ForkedDatabase {
    fn commit(&mut self, changes: Map<Address, Account>) {
        self.database_mut().commit(changes)
    }
}

/// Represents a snapshot of the database
#[derive(Debug)]
pub struct ForkDbSnapshot {
    local: CacheDB<SharedBackend>,
    accounts: BTreeMap<Address, AccountInfo>,
    storage: BTreeMap<Address, BTreeMap<U256, U256>>,
    block_hashes: BTreeMap<u64, H256>,
}

// === impl DbSnapshot ===

impl ForkDbSnapshot {
    fn get_storage(&self, address: Address, index: U256) -> Option<U256> {
        self.local.accounts.get(&address).and_then(|account| account.storage.get(&index)).copied()
    }
}

// This `DatabaseRef` implementation works similar to `CacheDB` which prioritizes modified elements,
// and uses another db as fallback
// We prioritize stored changed accounts/storage
impl DatabaseRef for ForkDbSnapshot {
    fn basic(&self, address: Address) -> AccountInfo {
        match self.local.accounts.get(&address) {
            Some(account) => account.info.clone(),
            None => {
                self.accounts.get(&address).cloned().unwrap_or_else(|| self.local.basic(address))
            }
        }
    }

    fn code_by_hash(&self, code_hash: H256) -> Bytecode {
        self.local.code_by_hash(code_hash)
    }

    fn storage(&self, address: Address, index: U256) -> U256 {
        match self.local.accounts.get(&address) {
            Some(account) => match account.storage.get(&index) {
                Some(entry) => *entry,
                None => self
                    .get_storage(address, index)
                    .unwrap_or_else(|| DatabaseRef::storage(&self.local, address, index)),
            },
            None => self
                .get_storage(address, index)
                .unwrap_or_else(|| DatabaseRef::storage(&self.local, address, index)),
        }
    }

    fn block_hash(&self, number: U256) -> H256 {
        self.block_hashes
            .get(&number.as_u64())
            .copied()
            .unwrap_or_else(|| self.local.block_hash(number))
    }
}
