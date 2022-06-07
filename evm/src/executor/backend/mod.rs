use crate::executor::{
    fork::{CreateFork, ForkId, MultiFork, SharedBackend},
    snapshot::Snapshots,
};
use bytes::Bytes;
use ethers::{
    prelude::{H160, H256, U256},
    types::Address,
};
use hashbrown::HashMap as Map;
use revm::{
    db::{CacheDB, DatabaseRef, EmptyDB},
    Account, AccountInfo, Database, DatabaseCommit, Env, Inspector, Log, Return, TransactOut,
};
use std::collections::HashMap;
use tracing::{trace, warn};
mod in_memory_db;
pub use in_memory_db::MemDb;

/// An extension trait that allows us to easily extend the `revm::Inspector` capabilities
#[auto_impl::auto_impl(&mut, Box)]
pub trait DatabaseExt: Database {
    /// Creates a new snapshot
    fn snapshot(&mut self) -> U256;
    /// Reverts the snapshot if it exists
    ///
    /// Returns `true` if the snapshot was successfully reverted, `false` if no snapshot for that id
    /// exists.
    fn revert(&mut self, id: U256) -> bool;

    /// Creates a new fork but does _not_ select it
    fn create_fork(&mut self, fork: CreateFork) -> eyre::Result<ForkId>;

    /// Selects the fork's state
    ///
    /// **Note**: this does not change the local state, but swaps the remote state
    ///
    /// # Errors
    ///
    /// Returns an error if no fork with the given `id` exists
    fn select_fork(&mut self, id: impl Into<ForkId>) -> eyre::Result<()>;
}

impl DatabaseExt for Backend {
    fn snapshot(&mut self) -> U256 {
        let id = self.snapshots.insert(self.db.clone());
        trace!(target: "backend", "Created new snapshot {}", id);
        id
    }

    fn revert(&mut self, id: U256) -> bool {
        if let Some(snapshot) = self.snapshots.remove(id) {
            self.db = snapshot;
            trace!(target: "backend", "Reverted snapshot {}", id);
            true
        } else {
            warn!(target: "backend", "No snapshot to revert for {}", id);
            false
        }
    }

    fn create_fork(&mut self, fork: CreateFork) -> eyre::Result<ForkId> {
        let (id, fork) = self.forks.create_fork(fork)?;
        self.created_forks.insert(id.clone(), fork);
        Ok(id)
    }

    fn select_fork(&mut self, id: impl Into<ForkId>) -> eyre::Result<()> {
        let id = id.into();
        let fork = self
            .created_forks
            .get(&id)
            .cloned()
            .ok_or_else(|| eyre::eyre!("Fork Id {} does not exist", id))?;
        *self.db.db_mut() = BackendDatabase::Forked(fork, id);
        Ok(())
    }
}

/// Provides the underlying `revm::Database` implementation.
///
/// A `Backend` can be initialised in two forms:
///
/// # 1. Empty in-memory Database
/// This is the default variant: an empty `revm::Database`
///
/// # 2. Forked Database
/// A `revm::Database` that forks off a remote client
///
///
/// In addition to that we support forking manually on the fly.
/// Additional forks can be created. Each unique fork is identified by its unique `ForkId`. We treat
/// forks as unique if they have the same `(endpoint, block number)` pair.
///
/// When it comes to testing, it's intended that each contract will use its own `Backend`
/// (`Backend::clone`). This way each contract uses its own encapsulated evm state. For in-memory
/// testing, the database is just an owned `revm::InMemoryDB`.
///
/// The `db` if fork-mode basically consists of 2 halves:
///   - everything fetched from the remote is readonly
///   - all local changes (instructed by the contract) are written to the backend's `db` and don't
///     alter the state of the remote client. This way a fork (`SharedBackend`), can be used by
///     multiple contracts at the same time.
///
/// # Fork swapping
///
/// Multiple "forks" can be created `Backend::create_fork()`, however only 1 can be used by the
/// `db`. However, their state can be hot-swapped by swapping the read half of `db` from one fork to
/// another.
///
/// **Note:** this only affects the readonly half of the `db`, local changes are persistent across
/// fork-state swaps.
///
/// # Snapshotting
///
/// A snapshot of the current overall state can be taken at any point in time. A snapshot is
/// identified by a unique id that's returned when a snapshot is created. A snapshot can only be
/// reverted _once_. After a successful revert, the same snapshot id cannot be used again. Reverting
/// a snapshot replaces the current active state with the snapshot state, the snapshot is deleted
/// afterwards, as well as any snapshots taken after the reverted snapshot, (e.g.: reverting to id
/// 0x1 will delete snapshots with ids 0x1, 0x2, etc.)
///
/// **Note:** Snapshots work across fork-swaps, e.g. if fork `A` is currently active, then a
/// snapshot is created before fork `B` is selected, then fork `A` will be the active fork again
/// after reverting the snapshot.
#[derive(Debug, Clone)]
pub struct Backend {
    /// The access point for managing forks
    forks: MultiFork,
    /// tracks all created forks
    created_forks: HashMap<ForkId, SharedBackend>,
    /// The database that holds the entire state, uses an internal database depending on current
    /// state
    pub db: CacheDB<BackendDatabase>,
    /// Contains snapshots made at a certain point
    snapshots: Snapshots<CacheDB<BackendDatabase>>,
}

// === impl Backend ===

impl Backend {
    /// Creates a new instance of `Backend`
    ///
    /// if `fork` is `Some` this will launch with a `fork` database, otherwise with an in-memory
    /// database
    pub fn new(forks: MultiFork, fork: Option<CreateFork>) -> Self {
        let db = if let Some(f) = fork {
            let (id, fork) = forks.create_fork(f).expect("Unable to fork");
            CacheDB::new(BackendDatabase::Forked(fork, id))
        } else {
            CacheDB::new(BackendDatabase::InMemory(EmptyDB()))
        };

        Self { forks, db, created_forks: Default::default(), snapshots: Default::default() }
    }

    /// Creates a new instance with a `BackendDatabase::InMemory` cache layer for the `CacheDB`
    pub fn clone_empty(&self) -> Self {
        let mut db = self.db.clone();
        *db.db_mut() = BackendDatabase::InMemory(EmptyDB());
        Self {
            forks: self.forks.clone(),
            created_forks: Default::default(),
            db,
            snapshots: Default::default(),
        }
    }

    pub fn insert_cache(&mut self, address: H160, account: AccountInfo) {
        self.db.insert_cache(address, account)
    }
}

// a bunch of delegate revm trait implementations

impl DatabaseRef for Backend {
    fn basic(&self, address: H160) -> AccountInfo {
        self.db.basic(address)
    }

    fn code_by_hash(&self, code_hash: H256) -> bytes::Bytes {
        self.db.code_by_hash(code_hash)
    }

    fn storage(&self, address: H160, index: U256) -> U256 {
        DatabaseRef::storage(&self.db, address, index)
    }

    fn block_hash(&self, number: U256) -> H256 {
        self.db.block_hash(number)
    }
}

impl DatabaseCommit for Backend {
    fn commit(&mut self, changes: Map<H160, Account>) {
        self.db.commit(changes)
    }
}

impl Database for Backend {
    fn basic(&mut self, address: H160) -> AccountInfo {
        self.db.basic(address)
    }

    fn code_by_hash(&mut self, code_hash: H256) -> Bytes {
        self.db.code_by_hash(code_hash)
    }

    fn storage(&mut self, address: H160, index: U256) -> U256 {
        Database::storage(&mut self.db, address, index)
    }

    fn block_hash(&mut self, number: U256) -> H256 {
        self.db.block_hash(number)
    }
}

/// Variants of a [revm::Database]
#[derive(Debug, Clone)]
pub enum BackendDatabase {
    /// Simple in-memory [revm::Database]
    InMemory(EmptyDB),
    /// A [revm::Database] that forks of a remote location and can have multiple consumers of the
    /// same data
    Forked(SharedBackend, ForkId),
}

impl DatabaseRef for BackendDatabase {
    fn basic(&self, address: H160) -> AccountInfo {
        match self {
            BackendDatabase::InMemory(inner) => inner.basic(address),
            BackendDatabase::Forked(inner, _) => inner.basic(address),
        }
    }

    fn code_by_hash(&self, address: H256) -> bytes::Bytes {
        match self {
            BackendDatabase::InMemory(inner) => inner.code_by_hash(address),
            BackendDatabase::Forked(inner, _) => inner.code_by_hash(address),
        }
    }

    fn storage(&self, address: H160, index: U256) -> U256 {
        match self {
            BackendDatabase::InMemory(inner) => inner.storage(address, index),
            BackendDatabase::Forked(inner, _) => inner.storage(address, index),
        }
    }

    fn block_hash(&self, number: U256) -> H256 {
        match self {
            BackendDatabase::InMemory(inner) => inner.block_hash(number),
            BackendDatabase::Forked(inner, _) => inner.block_hash(number),
        }
    }
}

/// A wrapper around `Backend` that ensures only `revm::DatabaseRef` functions are called.
///
/// Any changes made during its existence that affect the caching layer of the underlying Database
/// will result in a clone of the initial Database.
///
/// Main purpose for this type is for fuzzing. A test function fuzzer will repeatedly call the
/// function via immutable raw (no state changes).
///
/// **N.B.**: we're assuming cheatcodes that alter the state (like multi fork swapping) are niche.
/// If they executed during fuzzing, it will require a clone of the initial input database. This way
/// we can support these cheatcodes in fuzzing cheaply without adding overhead for fuzz tests that
/// don't make use of them. Alternatively each test case would require its own `Backend` clone,
/// which would add significant overhead for large fuzz sets even if the Database is not big after
/// setup.
pub(crate) struct FuzzBackendWrapper<'a> {
    pub inner: &'a Backend,
    /// active database clone that holds the currently active db, like reverted snapshots, selected
    /// fork, etc.
    db_override: Option<CacheDB<BackendDatabase>>,
    /// tracks all created forks
    created_forks: HashMap<ForkId, SharedBackend>,
    /// Contains snapshots made at a certain point
    snapshots: Snapshots<CacheDB<BackendDatabase>>,
}

// === impl RefBackendWrapper ===

impl<'a> FuzzBackendWrapper<'a> {
    pub fn new(inner: &'a Backend) -> Self {
        Self {
            inner,
            db_override: None,
            created_forks: Default::default(),
            snapshots: Default::default(),
        }
    }

    pub fn inspect_ref<INSP>(
        &mut self,
        mut env: Env,
        mut inspector: INSP,
    ) -> (Return, TransactOut, u64, Map<Address, Account>, Vec<Log>)
    where
        INSP: Inspector<Self>,
    {
        revm::evm_inner::<Self, true>(&mut env, self, &mut inspector).transact()
    }

    /// Returns the currently active database
    fn active_db(&self) -> &CacheDB<BackendDatabase> {
        self.db_override.as_ref().unwrap_or(&self.inner.db)
    }

    /// Sets the database override
    fn set_active(&mut self, db: CacheDB<BackendDatabase>) {
        self.db_override = Some(db)
    }
}

impl<'a> DatabaseExt for FuzzBackendWrapper<'a> {
    fn snapshot(&mut self) -> U256 {
        let id = self.snapshots.insert(self.active_db().clone());
        trace!(target: "backend", "Created new snapshot {}", id);
        id
    }

    fn revert(&mut self, id: U256) -> bool {
        if let Some(snapshot) =
            self.snapshots.remove(id).or_else(|| self.inner.snapshots.get(id).cloned())
        {
            self.set_active(snapshot);
            trace!(target: "backend", "Reverted snapshot {}", id);
            true
        } else {
            warn!(target: "backend", "No snapshot to revert for {}", id);
            false
        }
    }

    fn create_fork(&mut self, fork: CreateFork) -> eyre::Result<ForkId> {
        let (id, fork) = self.inner.forks.create_fork(fork)?;
        self.created_forks.insert(id.clone(), fork);
        Ok(id)
    }

    fn select_fork(&mut self, id: impl Into<ForkId>) -> eyre::Result<()> {
        let id = id.into();
        let fork = self
            .created_forks
            .get(&id)
            .or_else(|| self.inner.created_forks.get(&id))
            .cloned()
            .ok_or_else(|| eyre::eyre!("Fork Id {} does not exist", id))?;
        if let Some(ref mut db) = self.db_override {
            *db.db_mut() = BackendDatabase::Forked(fork, id);
        } else {
            let mut db = self.inner.db.clone();
            *db.db_mut() = BackendDatabase::Forked(fork, id);
            self.set_active(db);
        }
        Ok(())
    }
}

impl<'a> Database for FuzzBackendWrapper<'a> {
    fn basic(&mut self, address: H160) -> AccountInfo {
        if let Some(ref db) = self.db_override {
            DatabaseRef::basic(db, address)
        } else {
            DatabaseRef::basic(self.inner, address)
        }
    }
    fn code_by_hash(&mut self, code_hash: H256) -> Bytes {
        if let Some(ref db) = self.db_override {
            DatabaseRef::code_by_hash(db, code_hash)
        } else {
            DatabaseRef::code_by_hash(self.inner, code_hash)
        }
    }
    fn storage(&mut self, address: H160, index: U256) -> U256 {
        if let Some(ref db) = self.db_override {
            DatabaseRef::storage(db, address, index)
        } else {
            DatabaseRef::storage(self.inner, address, index)
        }
    }

    fn block_hash(&mut self, number: U256) -> H256 {
        if let Some(ref db) = self.db_override {
            DatabaseRef::block_hash(db, number)
        } else {
            DatabaseRef::block_hash(self.inner, number)
        }
    }
}
