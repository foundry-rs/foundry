//! The in memory DB
use crate::executor::backend::error::DatabaseError;
use ethers::{
    prelude::{H256, U256},
    types::Address,
};
use hashbrown::HashMap as Map;
use revm::{
    db::{CacheDB, DatabaseRef, EmptyDB},
    Account, AccountInfo, Bytecode, Database, DatabaseCommit,
};

use crate::executor::snapshot::Snapshots;

pub type InMemoryDB = CacheDB<EmptyDBWrapper>;

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
        Self { inner: CacheDB::new(Default::default()), snapshots: Default::default() }
    }
}

impl DatabaseRef for MemDb {
    type Error = DatabaseError;
    fn basic(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        DatabaseRef::basic(&self.inner, address)
    }

    fn code_by_hash(&self, code_hash: H256) -> Result<Bytecode, Self::Error> {
        DatabaseRef::code_by_hash(&self.inner, code_hash)
    }

    fn storage(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        DatabaseRef::storage(&self.inner, address, index)
    }

    fn block_hash(&self, number: U256) -> Result<H256, Self::Error> {
        DatabaseRef::block_hash(&self.inner, number)
    }
}

impl Database for MemDb {
    type Error = DatabaseError;
    #[track_caller]
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        Database::basic(&mut self.inner, address)
    }

    fn code_by_hash(&mut self, code_hash: H256) -> Result<Bytecode, Self::Error> {
        Database::code_by_hash(&mut self.inner, code_hash)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Database::storage(&mut self.inner, address, index)
    }

    fn block_hash(&mut self, number: U256) -> Result<H256, Self::Error> {
        Database::block_hash(&mut self.inner, number)
    }
}

impl DatabaseCommit for MemDb {
    fn commit(&mut self, changes: Map<Address, Account>) {
        DatabaseCommit::commit(&mut self.inner, changes)
    }
}

/// An empty database that always returns default values when queried.
///
/// This is just a simple wrapper for `revm::EmptyDB` but implements `DatabaseError` instead, this
/// way we can unify all different `Database` impls
#[derive(Debug, Default, Clone)]
pub struct EmptyDBWrapper(EmptyDB);

impl DatabaseRef for EmptyDBWrapper {
    type Error = DatabaseError;
    fn basic(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        Ok(self.0.basic(address)?)
    }
    fn code_by_hash(&self, code_hash: H256) -> Result<Bytecode, Self::Error> {
        Ok(self.0.code_by_hash(code_hash)?)
    }
    fn storage(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Ok(self.0.storage(address, index)?)
    }

    fn block_hash(&self, number: U256) -> Result<H256, Self::Error> {
        Ok(self.0.block_hash(number)?)
    }
}
