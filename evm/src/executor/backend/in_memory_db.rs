//! The in memory DB
use crate::executor::backend::{error::DatabaseError, DatabaseResult};
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

impl MemDb {
    /// the [`Database`](revm::Database) implementation for `CacheDB` manages an `AccountState` for the `DbAccount`, this will be set to `AccountState::NotExisting` if the account does not exist yet. This is because there's a distinction between "non-existing" and "empty", See <https://github.com/bluealloy/revm/blob/8f4348dc93022cffb3730d9db5d3ab1aad77676a/crates/revm/src/db/in_memory_db.rs#L81-L83>
    /// If an account is `NotExisting`, `Database(Ref)::basic` will always return `None` for the
    /// requested `AccountInfo`. To prevent
    ///
    /// This will ensure that a missing account is never marked as `NotExisting`
    fn ensure_loaded(&mut self, address: Address) -> DatabaseResult<AccountInfo> {
        if let Some(acc) = DatabaseRef::basic(self, address)? {
            Ok(acc)
        } else {
            self.inner.insert_account_info(address, Default::default());
            Ok(Default::default())
        }
    }
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

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.ensure_loaded(address).map(Some)
    }

    fn code_by_hash(&mut self, code_hash: H256) -> Result<Bytecode, Self::Error> {
        Database::code_by_hash(&mut self.inner, code_hash)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.ensure_loaded(address)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Ensures the `Database(Ref)` implementation for `revm::CacheDB` works as expected
    ///
    /// Demonstrates how calling `Database::basic` works if an account does not exist
    #[test]
    fn cache_db_insert_basic_non_existing() {
        let mut db = CacheDB::new(EmptyDBWrapper::default());
        let address = Address::random();

        // call `basic` on a non-existing account
        let info = Database::basic(&mut db, address).unwrap();
        assert!(info.is_none());
        let mut info = info.unwrap_or_default();
        info.balance = 500u64.into();

        // insert the modified account info
        db.insert_account_info(address, info);

        // when fetching again, the `AccountInfo` is still `None` because the state of the account is `AccountState::NotExisting`, See <https://github.com/bluealloy/revm/blob/8f4348dc93022cffb3730d9db5d3ab1aad77676a/crates/revm/src/db/in_memory_db.rs#L217-L226>
        let info = Database::basic(&mut db, address).unwrap();
        assert!(info.is_none());
    }

    /// Demonstrates how to insert a new account but not mark it as non-existing
    #[test]
    fn cache_db_insert_basic_default() {
        let mut db = CacheDB::new(EmptyDBWrapper::default());
        let address = Address::random();

        let info = DatabaseRef::basic(&db, address).unwrap();
        assert!(info.is_none());
        let mut info = info.unwrap_or_default();
        info.balance = 500u64.into();

        // insert the modified account info
        db.insert_account_info(address, info.clone());

        let loaded = Database::basic(&mut db, address).unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap(), info)
    }

    /// Demonstrates that `Database::basic` for `MemDb` will always return the `AccountInfo`
    #[test]
    fn mem_db_insert_basic_default() {
        let mut db = MemDb::default();
        let address = Address::random();

        let info = Database::basic(&mut db, address).unwrap();
        assert!(info.is_some());
        let mut info = info.unwrap();
        info.balance = 500u64.into();

        // insert the modified account info
        db.inner.insert_account_info(address, info.clone());

        let loaded = Database::basic(&mut db, address).unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap(), info)
    }
}
