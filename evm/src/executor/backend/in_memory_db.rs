//! The in memory DB

use bytes::Bytes;
use ethers::prelude::{H160, H256, U256};
use hashbrown::HashMap as Map;
use revm::{db::DatabaseRef, Account, AccountInfo, Database, DatabaseCommit, InMemoryDB};

use crate::executor::snapshot::Snapshots;

/// In memory Database for anvil
///
/// This acts like a wrapper type for [InMemoryDB] but is capable of applying snapshots
#[derive(Debug)]
pub struct MemDb {
    pub inner: InMemoryDB,
    pub snapshots: Snapshots<InMemoryDB>,
}

impl Default for MemDb {
    fn default() -> Self {
        Self { inner: InMemoryDB::default(), snapshots: Default::default() }
    }
}

impl DatabaseRef for MemDb {
    fn basic(&self, address: H160) -> AccountInfo {
        DatabaseRef::basic(&self.inner, address)
    }

    fn code_by_hash(&self, code_hash: H256) -> Bytes {
        DatabaseRef::code_by_hash(&self.inner, code_hash)
    }

    fn storage(&self, address: H160, index: U256) -> U256 {
        DatabaseRef::storage(&self.inner, address, index)
    }

    fn block_hash(&self, number: U256) -> H256 {
        DatabaseRef::block_hash(&self.inner, number)
    }
}

impl Database for MemDb {
    fn basic(&mut self, address: H160) -> AccountInfo {
        Database::basic(&mut self.inner, address)
    }

    fn code_by_hash(&mut self, code_hash: H256) -> Bytes {
        Database::code_by_hash(&mut self.inner, code_hash)
    }

    fn storage(&mut self, address: H160, index: U256) -> U256 {
        Database::storage(&mut self.inner, address, index)
    }

    fn block_hash(&mut self, number: U256) -> H256 {
        Database::block_hash(&mut self.inner, number)
    }
}

impl DatabaseCommit for MemDb {
    fn commit(&mut self, changes: Map<H160, Account>) {
        DatabaseCommit::commit(&mut self.inner, changes)
    }
}
