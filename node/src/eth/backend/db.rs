//! Helper types for working with [revm](foundry_evm::revm)

use crate::{
    revm::{Account, AccountInfo},
    U256,
};
use bytes::Bytes;
use ethers::prelude::{H160, H256};
use foundry_evm::{
    executor::{
        fork::{MemDb, SharedBackend},
        DatabaseRef,
    },
    revm::{db::CacheDB, Database, DatabaseCommit},
    HashMap as Map,
};
use std::sync::Arc;

/// This bundles all required revm traits
pub trait Db: DatabaseRef + Database + DatabaseCommit + Send + Sync + 'static {}

/// Implement the helper for revm's db
impl<ExtDB: DatabaseRef + Send + Sync + 'static> Db for CacheDB<ExtDB> {}

/// Implement the helper for the fork database
impl Db for ForkedDatabase {}

/// a [revm::Database] that's forked off another client
///
/// The `backend` is used to retrieve (missing) data, which is then fetched from the remote
/// endpoint. The inner in-memory database holds this storage and will be used for write operations.
/// This database uses the `backed` for read and the `db` for write operations. But note the
/// `backend` will also write (missing) data to the `db` in the background
#[derive(Debug)]
pub struct ForkedDatabase {
    /// responsible for fetching missing data
    ///
    /// This is respsonsible for getting data
    backend: SharedBackend,
    /// Contains all the data already fetched
    ///
    /// This is used for change commits
    db: Arc<MemDb>,
}

impl Database for ForkedDatabase {
    fn basic(&mut self, address: H160) -> AccountInfo {
        self.backend.basic(address)
    }

    fn code_by_hash(&mut self, code_hash: H256) -> Bytes {
        self.backend.code_by_hash(code_hash)
    }

    fn storage(&mut self, address: H160, index: U256) -> U256 {
        self.backend.storage(address, index)
    }

    fn block_hash(&mut self, number: U256) -> H256 {
        self.backend.block_hash(number)
    }
}

impl DatabaseRef for ForkedDatabase {
    fn basic(&self, address: H160) -> AccountInfo {
        self.backend.basic(address)
    }

    fn code_by_hash(&self, code_hash: H256) -> Bytes {
        self.backend.code_by_hash(code_hash)
    }

    fn storage(&self, address: H160, index: U256) -> U256 {
        self.backend.storage(address, index)
    }

    fn block_hash(&self, number: U256) -> H256 {
        self.backend.block_hash(number)
    }
}

impl DatabaseCommit for ForkedDatabase {
    fn commit(&mut self, changes: Map<H160, Account>) {
        self.db.do_commit(changes)
    }
}
