//! The in memory DB
use crate::{backend::error::DatabaseError, snapshot::Snapshots};
use alloy_primitives::{Address, B256, U256};
use revm::{
    db::{CacheDB, DatabaseRef, EmptyDB},
    primitives::{Account, AccountInfo, Bytecode, HashMap as Map},
    Database, DatabaseCommit,
};

/// Type alias for an in memory database
///
/// See `EmptyDBWrapper`
pub type FoundryEvmInMemoryDB = CacheDB<EmptyDBWrapper>;

/// In memory Database for anvil
///
/// This acts like a wrapper type for [InMemoryDB] but is capable of applying snapshots
#[derive(Debug)]
pub struct MemDb {
    pub inner: FoundryEvmInMemoryDB,
    pub snapshots: Snapshots<FoundryEvmInMemoryDB>,
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

    fn code_by_hash(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        DatabaseRef::code_by_hash(&self.inner, code_hash)
    }

    fn storage(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        DatabaseRef::storage(&self.inner, address, index)
    }

    fn block_hash(&self, number: U256) -> Result<B256, Self::Error> {
        DatabaseRef::block_hash(&self.inner, number)
    }
}

impl Database for MemDb {
    type Error = DatabaseError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        // Note: this will always return `Some(AccountInfo)`, See `EmptyDBWrapper`
        Database::basic(&mut self.inner, address)
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        Database::code_by_hash(&mut self.inner, code_hash)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Database::storage(&mut self.inner, address, index)
    }

    fn block_hash(&mut self, number: U256) -> Result<B256, Self::Error> {
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
///
/// This will also _always_ return `Some(AccountInfo)`:
///
/// The [`Database`](revm::Database) implementation for `CacheDB` manages an `AccountState` for the
/// `DbAccount`, this will be set to `AccountState::NotExisting` if the account does not exist yet.
/// This is because there's a distinction between "non-existing" and "empty",
/// see <https://github.com/bluealloy/revm/blob/8f4348dc93022cffb3730d9db5d3ab1aad77676a/crates/revm/src/db/in_memory_db.rs#L81-L83>.
/// If an account is `NotExisting`, `Database(Ref)::basic` will always return `None` for the
/// requested `AccountInfo`. To prevent this, we ensure that a missing account is never marked as
/// `NotExisting` by always returning `Some` with this type.
#[derive(Debug, Default, Clone)]
pub struct EmptyDBWrapper(EmptyDB);

impl DatabaseRef for EmptyDBWrapper {
    type Error = DatabaseError;

    fn basic(&self, _address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        // Note: this will always return `Some(AccountInfo)`, for the reason explained above
        Ok(Some(AccountInfo::default()))
    }

    fn code_by_hash(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        Ok(self.0.code_by_hash(code_hash)?)
    }
    fn storage(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Ok(self.0.storage(address, index)?)
    }

    fn block_hash(&self, number: U256) -> Result<B256, Self::Error> {
        Ok(self.0.block_hash(number)?)
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::b256;

    use super::*;

    /// Ensures the `Database(Ref)` implementation for `revm::CacheDB` works as expected
    ///
    /// Demonstrates how calling `Database::basic` works if an account does not exist
    #[test]
    fn cache_db_insert_basic_non_existing() {
        let mut db = CacheDB::new(EmptyDB::default());
        let address = Address::random();
        // call `basic` on a non-existing account
        let info = Database::basic(&mut db, address).unwrap();
        assert!(info.is_none());
        let mut info = info.unwrap_or_default();
        info.balance = U256::from(500u64);

        // insert the modified account info
        db.insert_account_info(address, info);

        // when fetching again, the `AccountInfo` is still `None` because the state of the account
        // is `AccountState::NotExisting`, see <https://github.com/bluealloy/revm/blob/8f4348dc93022cffb3730d9db5d3ab1aad77676a/crates/revm/src/db/in_memory_db.rs#L217-L226>
        let info = Database::basic(&mut db, address).unwrap();
        assert!(info.is_none());
    }

    /// Demonstrates how to insert a new account but not mark it as non-existing
    #[test]
    fn cache_db_insert_basic_default() {
        let mut db = CacheDB::new(EmptyDB::default());
        let address = Address::random();

        let info = DatabaseRef::basic(&db, address).unwrap();
        assert!(info.is_none());
        let mut info = info.unwrap_or_default();
        info.balance = U256::from(500u64);

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
        let address = Address::from_word(b256!(
            "000000000000000000000000d8da6bf26964af9d7eed9e03e53415d37aa96045"
        ));

        let info = Database::basic(&mut db, address).unwrap();
        assert!(info.is_some());
        let mut info = info.unwrap();
        info.balance = U256::from(500u64);

        // insert the modified account info
        db.inner.insert_account_info(address, info.clone());

        let loaded = Database::basic(&mut db, address).unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap(), info)
    }
}
