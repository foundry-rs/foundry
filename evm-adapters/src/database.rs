//! Bare bones Database and storage types

use ethers::types::{Address, H256};
use std::{collections::HashMap, sync::RwLock};

/// The used to differentiate database columns
pub type Column = u32;

/// common column identifiers
pub mod columns {
    use crate::database::Column;

    pub const STATE: Column = 1;
}

/// Different operations applicable to a Database
#[derive(Clone)]
pub enum Operation {
    Set(Column, Address, H256, H256),
    Remove(Column, Address, H256),
    Store(Column, Address, H256, H256),
    Reference(Column, Address, H256),
}

/// A series of changes to the database that can be committed atomically. They do not take effect
/// until passed into `Database::write`.
#[derive(Default, Clone)]
pub struct Transaction(pub Vec<Operation>);

/// General Key-Value abstraction over the underlying database
///
/// A Key-Value database uses "column families", which are like distinct stores within a database.
/// A key written in one particular column will not we found in any other.
pub trait Database: Send + Sync {
    /// Get a value by key.
    fn get(&self, col: Column, key: &[u8]) -> Option<Vec<u8>>;

    /// Write a transaction of changes to the underlying store.
    fn write(&self, transaction: Transaction) -> eyre::Result<()>;
}

type MemDb = RwLock<HashMap<Column, HashMap<Vec<u8>, (u32, Vec<u8>)>>>;

/// Database as an in-memory hash map.
#[derive(Default)]
pub struct MemoryDb(MemDb);

/// Storage key that keeps track of reads/writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedStorageKey {
    pub key: H256,
    pub reads: u32,
    pub writes: u32,
}
