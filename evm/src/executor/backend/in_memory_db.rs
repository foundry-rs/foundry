//! The in memory DB
use crate::executor::{backend::error::DatabaseError, fork::database::DbResult};
use ethers::{
    prelude::{H256, U256},
    types::Address,
};
use hashbrown::HashMap as Map;
use revm::{db::DatabaseRef, Account, AccountInfo, Bytecode, Database, DatabaseCommit, InMemoryDB};

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
    type Error = DatabaseError;
    fn basic(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        Ok(DatabaseRef::basic(&self.inner, address).unwrap())
    }

    fn code_by_hash(&self, code_hash: H256) -> Result<Bytecode, Self::Error> {
        Ok(DatabaseRef::code_by_hash(&self.inner, code_hash).unwrap())
    }

    fn storage(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Ok(DatabaseRef::storage(&self.inner, address, index).unwrap())
    }

    fn block_hash(&self, number: U256) -> Result<H256, Self::Error> {
        Ok(DatabaseRef::block_hash(&self.inner, number).unwrap())
    }
}

impl Database for MemDb {
    type Error = DatabaseError;
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        Ok(Database::basic(&mut self.inner, address).unwrap())
    }

    fn code_by_hash(&mut self, code_hash: H256) -> Result<Bytecode, Self::Error> {
        Ok(Database::code_by_hash(&mut self.inner, code_hash).unwrap())
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Ok(Database::storage(&mut self.inner, address, index).unwrap())
    }

    fn block_hash(&mut self, number: U256) -> Result<H256, Self::Error> {
        Ok(Database::block_hash(&mut self.inner, number).unwrap())
    }
}

impl DatabaseCommit for MemDb {
    fn commit(&mut self, changes: Map<Address, Account>) {
        DatabaseCommit::commit(&mut self.inner, changes)
    }
}
