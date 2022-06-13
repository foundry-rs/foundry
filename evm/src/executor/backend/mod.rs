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
    Account, AccountInfo, Database, DatabaseCommit, Env, Inspector, Log, Return, SubRoutine,
    TransactOut,
};
use std::collections::HashMap;
use tracing::{trace, warn};
mod fuzz;
mod snapshot;
pub use fuzz::FuzzBackendWrapper;
mod in_memory_db;
use crate::executor::backend::snapshot::BackendSnapshot;
pub use in_memory_db::MemDb;

/// An extension trait that allows us to easily extend the `revm::Inspector` capabilities
#[auto_impl::auto_impl(&mut, Box)]
pub trait DatabaseExt: Database {
    /// Creates a new snapshot
    fn snapshot(&mut self, subroutine: &SubRoutine) -> U256;
    /// Reverts the snapshot if it exists
    ///
    /// Returns `true` if the snapshot was successfully reverted, `false` if no snapshot for that id
    /// exists.
    fn revert(&mut self, id: U256) -> Option<SubRoutine>;

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
    snapshots: Snapshots<BackendSnapshot<CacheDB<BackendDatabase>>>,
}

// === impl Backend ===

impl Backend {
    /// Creates a new Backend with a spawned multi fork thread
    pub fn spawn(fork: Option<CreateFork>) -> Self {
        Self::new(MultiFork::spawn(), fork)
    }

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

    /// Executes the configured transaction of the `env` without commiting state changes
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
}

// === impl a bunch of `revm::Database` adjacent implementations ===

impl DatabaseExt for Backend {
    fn snapshot(&mut self, subroutine: &SubRoutine) -> U256 {
        let id = self.snapshots.insert(BackendSnapshot::new(self.db.clone(), subroutine.clone()));
        trace!(target: "backend", "Created new snapshot {}", id);
        id
    }

    fn revert(&mut self, id: U256) -> Option<SubRoutine> {
        if let Some(BackendSnapshot { db, subroutine }) = self.snapshots.remove(id) {
            self.db = db;
            trace!(target: "backend", "Reverted snapshot {}", id);
            Some(subroutine)
        } else {
            warn!(target: "backend", "No snapshot to revert for {}", id);
            None
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

impl<'a> DatabaseRef for &'a mut Backend {
    fn basic(&self, address: H160) -> AccountInfo {
        DatabaseRef::basic(&self.db, address)
    }

    fn code_by_hash(&self, code_hash: H256) -> bytes::Bytes {
        DatabaseRef::code_by_hash(&self.db, code_hash)
    }

    fn storage(&self, address: H160, index: U256) -> U256 {
        DatabaseRef::storage(&self.db, address, index)
    }

    fn block_hash(&self, number: U256) -> H256 {
        DatabaseRef::block_hash(&self.db, number)
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
