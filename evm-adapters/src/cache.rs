//! Cache types

use cached::SizedCache;
use ethers::types::{Address, BlockNumber, H256, U64};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, RwLock},
};

type AccountStorageKey = (Address, H256);

/// A list of storage values.
pub type StorageList = Vec<(AccountStorageKey, Option<H256>)>;

/// A basic state cache.
pub struct Cache {
    /// A sized storage cache.
    /// A `None` value indicates that the kex is known to be missing
    // TODO this should probably track the ~size in bytes rather then number of entries
    storage_cache: SizedCache<AccountStorageKey, Option<H256>>,

    /// Recent block modifications
    // TODO might be overhead at this point
    modifications: VecDeque<BlockStorageChanges>,
}

impl Cache {
    pub fn new(size: usize) -> Self {
        Self { storage_cache: SizedCache::with_size(size), modifications: Default::default() }
    }

    pub fn into_shared(self) -> SharedCache {
        Arc::new(RwLock::new(self))
    }
}

/// A state cache that can be shared across threads
///
/// This can can be used as global state cache.
pub type SharedCache = Arc<RwLock<Cache>>;

/// Stores state values locally.
pub struct LocalStateCache {
    /// Storage cache.
    storage: HashMap<AccountStorageKey, Option<H256>>,
}

/// A change aware abstraction over a local and shared state cache.
///
/// Manages the global state and can sync the local cache
pub struct StateChangeCache {
    /// Shared global state cache.
    shared_cache: SharedCache,
    /// Cache of local values for this state.
    local_cache: RwLock<LocalStateCache>,
}

impl StateChangeCache {
    /// Synchronizes the local cache into the shared cache
    pub fn sync(&mut self, changes: StorageList) {
        todo!()
    }
}

/// A set of storage slots changed in within a block.
#[derive(Debug)]
struct BlockStorageChanges {
    /// Block number
    number: U64,
    // The block's hash
    hash: H256,
    /// The parent block hash
    parent: H256,
    /// The modified storage entries
    storage: HashSet<AccountStorageKey>,
}

/// The state backend of the evm is used to read and write storage
// TODO unify with EVM adapter specific traits like `StackState`, `Backend`
pub trait StateBackend {
    /// Error type that's thrown when fetching data failed
    type Error;

    /// Get storage value of address at index.
    fn storage(&self, address: Address, index: H256) -> Result<Option<H256>, Self::Error>;

    fn write_storage(&self, address: Address, key: H256, value: H256) -> Result<(), Self::Error>;
}

// TODO impl StateBackend for StateChangeCache {}
