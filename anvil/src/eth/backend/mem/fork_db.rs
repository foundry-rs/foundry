use crate::{
    eth::{backend::db::Db, error::BlockchainError},
    mem::snapshot::Snapshots,
    revm::{db::DatabaseRef, Account, AccountInfo, Database, DatabaseCommit},
    Address, U256,
};
use ethers::prelude::H256;
use forge::HashMap as Map;
use foundry_evm::executor::fork::{BlockchainDb, SharedBackend};
use parking_lot::Mutex;
use std::{collections::BTreeMap, sync::Arc};
use tracing::{trace, warn};

/// Implement the helper for the fork database
impl Db for ForkedDatabase {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.db.db().do_insert_account(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: U256, val: U256) {
        let mut db = self.db.db().storage.write();
        db.entry(address).or_default().insert(slot, val);
    }

    fn snapshot(&mut self) -> U256 {
        let db = self.db.db();
        let snapshot = DbSnapshot {
            accounts: db.accounts.read().clone(),
            storage: db.storage.read().clone(),
            block_hashes: db.block_hashes.read().clone(),
        };
        let mut snapshots = self.snapshots.lock();
        let id = snapshots.insert(snapshot);
        trace!(target: "backend::forkdb", "Created new snapshot {}", id);
        id
    }

    fn revert(&mut self, id: U256) -> bool {
        let snapshot = { self.snapshots.lock().remove(id) };
        if let Some(snapshot) = snapshot {
            let DbSnapshot { accounts, storage, block_hashes } = snapshot;
            let db = self.db.db();
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

            trace!(target: "backend::forkdb", "Reverted snapshot {}", id);
            true
        } else {
            warn!(target: "backend::forkdb", "No snapshot to revert for {}", id);
            false
        }
    }
}

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
    /// Contains all the data already fetched
    ///
    /// This is used for change commits
    db: BlockchainDb,
    /// holds the snapshot state of a blockchain
    snapshots: Arc<Mutex<Snapshots<DbSnapshot>>>,
}

impl ForkedDatabase {
    /// Creates a new instance of this DB
    pub fn new(backend: SharedBackend, db: BlockchainDb) -> Self {
        Self { backend, db, snapshots: Arc::new(Mutex::new(Default::default())) }
    }

    /// Reset the fork to a fresh forked state, and optionally update the fork config
    pub fn reset(
        &self,
        _url: Option<String>,
        block_number: Option<u64>,
    ) -> Result<(), BlockchainError> {
        if let Some(block_number) = block_number {
            self.backend
                .set_pinned_block(block_number)
                .map_err(|err| BlockchainError::Internal(err.to_string()))?;
        }

        // TODO need to find a way to update generic provider via url

        self.db.db().clear();
        trace!(target: "backend::forkdb", "Cleared database");
        Ok(())
    }

    /// Flushes the cache to disk if configured
    pub fn flush_cache(&self) {
        self.db.cache().flush()
    }
}

impl Database for ForkedDatabase {
    fn basic(&mut self, address: Address) -> AccountInfo {
        self.backend.basic(address)
    }

    fn code_by_hash(&mut self, code_hash: H256) -> bytes::Bytes {
        self.backend.code_by_hash(code_hash)
    }

    fn storage(&mut self, address: Address, index: U256) -> U256 {
        self.backend.storage(address, index)
    }

    fn block_hash(&mut self, number: U256) -> H256 {
        self.backend.block_hash(number)
    }
}

impl DatabaseRef for ForkedDatabase {
    fn basic(&self, address: Address) -> AccountInfo {
        self.backend.basic(address)
    }

    fn code_by_hash(&self, code_hash: H256) -> bytes::Bytes {
        self.backend.code_by_hash(code_hash)
    }

    fn storage(&self, address: Address, index: U256) -> U256 {
        self.backend.storage(address, index)
    }

    fn block_hash(&self, number: U256) -> H256 {
        self.backend.block_hash(number)
    }
}

impl DatabaseCommit for ForkedDatabase {
    fn commit(&mut self, changes: Map<Address, Account>) {
        self.db.db().do_commit(changes)
    }
}

/// Represents a snapshot of the database
#[derive(Debug)]
struct DbSnapshot {
    accounts: BTreeMap<Address, AccountInfo>,
    storage: BTreeMap<Address, BTreeMap<U256, U256>>,
    block_hashes: BTreeMap<u64, H256>,
}
