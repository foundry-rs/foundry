//! The database Backend

use foundry_evm::{
    executor::{fork::BlockchainDbMeta, DatabaseRef},
    revm::{db::CacheDB, Database, DatabaseCommit},
};
use parking_lot::Mutex;
use std::sync::Arc;

/// Gives access to the [revm::Database]
#[derive(Clone)]
pub struct Backend {
    /// access to revm's database related operations
    db: Arc<dyn Db>,
    /// The lock used to gain exclusive access when doing write operations on the DB
    write_lock: Arc<Mutex<()>>,
    /// meta data of the chain
    meta: Arc<BlockchainDbMeta>,
}

/// This bundles all required revm traits
trait Db: DatabaseRef + Database + DatabaseCommit + Send + Sync {}

impl<ExtDB: DatabaseRef + Send + Sync> Db for CacheDB<ExtDB> {}
