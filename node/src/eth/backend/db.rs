//! Helper types for working with [revm](foundry_evm::revm)

use bytes::Bytes;
use ethers::prelude::{H160, H256, U256};
use foundry_evm::{
    executor::DatabaseRef,
    revm::{db::CacheDB, AccountInfo, Database, DatabaseCommit},
};

/// This bundles all required revm traits
pub trait Db: DatabaseRef + Database + DatabaseCommit + Send + Sync {}

// no auto_impl for &mut DatabaseRef but need to implement to satisfy `Db` trait
impl<'a> DatabaseRef for &'a mut (dyn Db + 'a) {
    fn basic(&self, address: H160) -> AccountInfo {
        <dyn Db as DatabaseRef>::basic(self, address)
    }

    fn code_by_hash(&self, code_hash: H256) -> Bytes {
        <dyn Db as DatabaseRef>::code_by_hash(self, code_hash)
    }

    fn storage(&self, address: H160, index: U256) -> U256 {
        <dyn Db as DatabaseRef>::storage(self, address, index)
    }

    fn block_hash(&self, number: U256) -> H256 {
        <dyn Db as DatabaseRef>::block_hash(self, number)
    }
}

// Blanket impl for mutable dyn references
impl<'a> Db for &'a mut (dyn Db + 'a) {}

/// Implement the helper
impl<ExtDB: DatabaseRef + Send + Sync> Db for CacheDB<ExtDB> {}
