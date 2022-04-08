//! Helper types for working with [revm](foundry_evm::revm)

use foundry_evm::{
    executor::{DatabaseRef},
    revm::{db::CacheDB, Database, DatabaseCommit},
};

/// This bundles all required revm traits
pub trait Db: DatabaseRef + Database + DatabaseCommit + Send + Sync + 'static {}

/// Implement the helper
impl<ExtDB: DatabaseRef + Send + Sync + 'static> Db for CacheDB<ExtDB> {}

// impl Db for SharedBackend {}
