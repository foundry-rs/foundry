//! Helper types for working with [revm](foundry_evm::revm)

use bytes::Bytes;
use ethers::prelude::{H160, H256, U256};
use foundry_evm::{
    executor::DatabaseRef,
    revm::{db::CacheDB, AccountInfo, Database, DatabaseCommit},
};

/// This bundles all required revm traits
pub trait Db: DatabaseRef + Database + DatabaseCommit + Send + Sync + 'static {}

/// Implement the helper
impl<ExtDB: DatabaseRef + Send + Sync + 'static> Db for CacheDB<ExtDB> {}
