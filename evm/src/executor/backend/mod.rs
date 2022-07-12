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
    TransactOut, TransactTo,
};
use std::collections::HashMap;
use tracing::{trace, warn};
mod fuzz;
mod snapshot;
pub use fuzz::FuzzBackendWrapper;
mod in_memory_db;
use crate::{abi::CHEATCODE_ADDRESS, executor::backend::snapshot::BackendSnapshot};
pub use in_memory_db::MemDb;

/// An extension trait that allows us to easily extend the `revm::Inspector` capabilities
#[auto_impl::auto_impl(&mut, Box)]
pub trait DatabaseExt: Database {
    /// Creates a new snapshot at the current point of execution.
    ///
    /// A snapshot is associated with a new unique id that's created for the snapshot.
    /// Snapshots can be reverted: [DatabaseExt::revert], however a snapshot can only be reverted
    /// once. After a successful revert, the same snapshot id cannot be used again.
    fn snapshot(&mut self, subroutine: &SubRoutine, env: &Env) -> U256;
    /// Reverts the snapshot if it exists
    ///
    /// Returns `true` if the snapshot was successfully reverted, `false` if no snapshot for that id
    /// exists.
    ///
    /// **N.B.** While this reverts the state of the evm to the snapshot, it keeps new logs made
    /// since the snapshots was created. This way we can show logs that were emitted between
    /// snapshot and its revert.
    /// This will also revert any changes in the `Env` and replace it with the caputured `Env` of
    /// `Self::snapshot`
    fn revert(&mut self, id: U256, subroutine: &SubRoutine, env: &mut Env) -> Option<SubRoutine>;

    /// Creates and also selects a new fork
    ///
    /// This is basically `create_fork` + `select_fork`
    fn create_select_fork(&mut self, fork: CreateFork, env: &mut Env) -> eyre::Result<U256> {
        let id = self.create_fork(fork)?;
        self.select_fork(id, env)?;
        Ok(id)
    }

    /// Creates a new fork but does _not_ select it
    fn create_fork(&mut self, fork: CreateFork) -> eyre::Result<U256>;

    /// Selects the fork's state
    ///
    /// This will also modify the current `Env`.
    ///
    /// **Note**: this does not change the local state, but swaps the remote state
    ///
    /// # Errors
    ///
    /// Returns an error if no fork with the given `id` exists
    fn select_fork(&mut self, id: U256, env: &mut Env) -> eyre::Result<()>;

    /// Updates the fork to given block number.
    ///
    /// This will essentially create a new fork at the given block height.
    ///
    /// # Errors
    ///
    /// Returns an error if not matching fork was found.
    fn roll_fork(
        &mut self,
        env: &mut Env,
        block_number: U256,
        id: Option<U256>,
    ) -> eyre::Result<()>;

    /// Returns the `ForkId` that's currently used in the database, if fork mode is on
    fn active_fork(&self) -> Option<U256>;

    /// Ensures that an appropriate fork exits
    ///
    /// If `id` contains a requested `Fork` this will ensure it exits.
    /// Otherwise this returns the currently active fork.
    ///
    /// # Errors
    ///
    /// Returns an error if the given `id` does not match any forks
    ///
    /// Returns an error if no fork exits
    fn ensure_fork(&self, id: Option<U256>) -> eyre::Result<U256>;

    /// Ensures that a corresponding `ForkId` exists for the given local `id`
    fn ensure_fork_id(&self, id: U256) -> eyre::Result<&ForkId>;
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
/// When swapping forks (`Backend::select_fork()`) we also update the current `Env` of the `EVM`
/// accordingly, so that all `block.*` config values match
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
    /// The database that holds the entire state, uses an internal database depending on current
    /// state
    pub db: CacheDB<BackendDatabase>,
    /// holds additional Backend data
    inner: BackendInner,
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
        let (db, launched_with) = if let Some(f) = fork {
            let (id, fork) = forks.create_fork(f).expect("Unable to fork");
            (CacheDB::new(BackendDatabase::Forked(fork.clone(), U256::zero())), Some((id, fork)))
        } else {
            (CacheDB::new(BackendDatabase::InMemory(EmptyDB())), None)
        };
        // Note: this will take of registering the `fork`
        Self { forks, db, inner: BackendInner::new(launched_with) }
    }

    /// Creates a new instance with a `BackendDatabase::InMemory` cache layer for the `CacheDB`
    pub fn clone_empty(&self) -> Self {
        let mut db = self.db.clone();
        db.db = BackendDatabase::InMemory(EmptyDB());
        Self { forks: self.forks.clone(), db, inner: Default::default() }
    }

    pub fn insert_account_info(&mut self, address: H160, account: AccountInfo) {
        self.db.insert_account_info(address, account)
    }

    /// Returns all forks created by this backend
    pub fn created_forks(&self) -> &HashMap<ForkId, SharedBackend> {
        &self.inner.created_forks
    }

    /// Returns all snapshots created in this backend
    pub fn snapshots(&self) -> &Snapshots<BackendSnapshot<CacheDB<BackendDatabase>>> {
        &self.inner.snapshots
    }

    /// Sets the address of the `DSTest` contract that is being executed
    pub fn set_test_contract(&mut self, addr: Address) -> &mut Self {
        self.inner.test_contract_context = Some(addr);
        self
    }

    /// Returns the address of the set `DSTest` contract
    pub fn test_contract_address(&self) -> Option<Address> {
        self.inner.test_contract_context
    }

    /// Checks if the test contract associated with this backend failed, See
    /// [Self::is_failed_test_contract]
    pub fn is_failed(&self) -> bool {
        self.inner.has_failure_snapshot ||
            self.test_contract_address()
                .map(|addr| self.is_failed_test_contract(addr))
                .unwrap_or_default()
    }

    /// Checks if the given test function failed
    ///
    /// DSTest will not revert inside its `assertEq`-like functions which allows
    /// to test multiple assertions in 1 test function while also preserving logs.
    /// Instead, it stores whether an `assert` failed in a boolean variable that we can read
    pub fn is_failed_test_contract(&self, address: Address) -> bool {
        /*
         contract DSTest {
            bool public IS_TEST = true;
            // slot 0 offset 1 => second byte of slot0
            bool private _failed;
         }
        */
        let value = self.storage(address, U256::zero());

        value.byte(1) != 0
    }

    /// In addition to the `_failed` variable, `DSTest::fail()` stores a failure
    /// in "failed"
    /// See <https://github.com/dapphub/ds-test/blob/9310e879db8ba3ea6d5c6489a579118fd264a3f5/src/test.sol#L66-L72>
    pub fn is_global_failure(&self) -> bool {
        let index = U256::from(&b"failed"[..]);
        let value = self.storage(CHEATCODE_ADDRESS, index);
        value == U256::one()
    }

    /// Executes the configured test call of the `env` without commiting state changes
    pub fn inspect_ref<INSP>(
        &mut self,
        mut env: Env,
        mut inspector: INSP,
    ) -> (Return, TransactOut, u64, Map<Address, Account>, Vec<Log>)
    where
        INSP: Inspector<Self>,
    {
        if let TransactTo::Call(to) = env.tx.transact_to {
            self.inner.test_contract_context = Some(to);
        }
        revm::evm_inner::<Self, true>(&mut env, self, &mut inspector).transact()
    }
}

// === impl a bunch of `revm::Database` adjacent implementations ===

impl DatabaseExt for Backend {
    fn snapshot(&mut self, subroutine: &SubRoutine, env: &Env) -> U256 {
        let id = self.inner.snapshots.insert(BackendSnapshot::new(
            self.db.clone(),
            subroutine.clone(),
            env.clone(),
        ));
        trace!(target: "backend", "Created new snapshot {}", id);
        id
    }

    fn revert(
        &mut self,
        id: U256,
        subroutine: &SubRoutine,
        current: &mut Env,
    ) -> Option<SubRoutine> {
        if let Some(mut snapshot) = self.inner.snapshots.remove(id) {
            // need to check whether DSTest's `failed` variable is set to `true` which means an
            // error occurred either during the snapshot or even before
            if self.is_failed() {
                self.inner.has_failure_snapshot = true;
            }

            // merge additional logs
            snapshot.merge(subroutine);
            let BackendSnapshot { db, subroutine, env } = snapshot;
            self.db = db;
            update_current_env_with_fork_env(current, env);

            trace!(target: "backend", "Reverted snapshot {}", id);
            Some(subroutine)
        } else {
            warn!(target: "backend", "No snapshot to revert for {}", id);
            None
        }
    }

    fn create_fork(&mut self, fork: CreateFork) -> eyre::Result<U256> {
        let (id, fork) = self.forks.create_fork(fork)?;
        let id = self.inner.insert_new_fork(id, fork);
        Ok(id)
    }

    fn select_fork(&mut self, id: U256, env: &mut Env) -> eyre::Result<()> {
        let fork_id = self.ensure_fork_id(id).cloned()?;
        let fork_env = self
            .forks
            .get_env(fork_id)?
            .ok_or_else(|| eyre::eyre!("Requested fork `{}` does not exit", id))?;
        let fork = self.inner.ensure_backend(id).cloned()?;
        self.db.db = BackendDatabase::Forked(fork, id);
        update_current_env_with_fork_env(env, fork_env);
        Ok(())
    }

    fn roll_fork(
        &mut self,
        env: &mut Env,
        block_number: U256,
        id: Option<U256>,
    ) -> eyre::Result<()> {
        let id = self.ensure_fork(id)?;
        let (fork_id, fork) =
            self.forks.roll_fork(self.inner.ensure_fork_id(id).cloned()?, block_number.as_u64())?;
        // this will update the local mapping
        self.inner.update_fork_mapping(id, fork_id, fork);
        if self.active_fork() == Some(id) {
            // need to update the block number right away
            env.block.number = block_number;
        }
        Ok(())
    }

    fn active_fork(&self) -> Option<U256> {
        self.db.db.as_fork()
    }

    fn ensure_fork(&self, id: Option<U256>) -> eyre::Result<U256> {
        if let Some(id) = id {
            if self.inner.issued_local_fork_ids.contains_key(&id) {
                return Ok(id)
            }
            eyre::bail!("Requested fork `{}` does not exit", id)
        }
        if let Some(id) = self.active_fork() {
            Ok(id)
        } else {
            eyre::bail!("No fork active")
        }
    }

    fn ensure_fork_id(&self, id: U256) -> eyre::Result<&ForkId> {
        self.inner.ensure_fork_id(id)
    }
}

impl DatabaseRef for Backend {
    fn basic(&self, address: H160) -> AccountInfo {
        self.db.basic(address)
    }

    fn code_by_hash(&self, code_hash: H256) -> Bytes {
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

    fn code_by_hash(&self, code_hash: H256) -> Bytes {
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
    Forked(SharedBackend, U256),
}

// === impl BackendDatabase ===

impl BackendDatabase {
    /// Returns the `ForkId` if in fork mode
    pub fn as_fork(&self) -> Option<U256> {
        match self {
            BackendDatabase::InMemory(_) => None,
            BackendDatabase::Forked(_, id) => Some(*id),
        }
    }
}

impl DatabaseRef for BackendDatabase {
    fn basic(&self, address: H160) -> AccountInfo {
        match self {
            BackendDatabase::InMemory(inner) => inner.basic(address),
            BackendDatabase::Forked(inner, _) => inner.basic(address),
        }
    }

    fn code_by_hash(&self, address: H256) -> Bytes {
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

/// Container type for various Backend related data
#[derive(Debug, Clone, Default)]
pub struct BackendInner {
    /// Stores the `ForkId` of the fork the `Backend` launched with from the start.
    ///
    /// In other words if [`Backend::spawn()`] was called with a `CreateFork` command, to launch
    /// directly in fork mode, this holds the corresponding `ForkId` of this fork.
    pub launched_with_fork: Option<ForkId>,
    /// This tracks numeric fork ids and the `ForkId` used by the handler.
    ///
    /// This is neceessary, because there can be multiple `Backends` associated with a single
    /// `ForkId` which is only a pair of endpoint + block. Since an existing fork can be
    /// modified (e.g. `roll_fork`), but this should only affect the fork that's unique for the
    /// test and not the `ForkId`
    ///
    /// This ensures we can treat forks as unique from the context of a test, so rolling to another
    /// is basically creating(or reusing) another `ForkId` that's then mapped to the previous
    /// issued _local_ numeric identifier, that remains constant, even if the underlying fork
    /// backend changes.
    pub issued_local_fork_ids: HashMap<U256, ForkId>,
    /// tracks all created forks
    pub created_forks: HashMap<ForkId, SharedBackend>,
    /// Contains snapshots made at a certain point
    pub snapshots: Snapshots<BackendSnapshot<CacheDB<BackendDatabase>>>,
    /// Tracks whether there was a failure in a snapshot that was reverted
    ///
    /// The Test contract contains a bool variable that is set to true when an `assert` function
    /// failed. When a snapshot is reverted, it reverts the state of the evm, but we still want
    /// to know if there was an `assert` that failed after the snapshot was taken so that we can
    /// check if the test function passed all asserts even across snapshots. When a snapshot is
    /// reverted we get the _current_ `revm::Subroutine` which contains the state that we can check
    /// if the `_failed` variable is set,
    /// additionally
    pub has_failure_snapshot: bool,
    /// Tracks the address of a Test contract
    ///
    /// This address can be used to inspect the state of the contract when a test is being
    /// executed. E.g. the `_failed` variable of `DSTest`
    pub test_contract_context: Option<Address>,
    /// Tracks numeric identifiers for forks
    pub next_fork_id: U256,
}

// === impl BackendInner ===

impl BackendInner {
    /// Creates a new instance that tracks the fork used at launch
    pub fn new(launched_with: Option<(ForkId, SharedBackend)>) -> Self {
        launched_with.map(|(id, backend)| Self::forked(id, backend)).unwrap_or_default()
    }

    pub fn forked(id: ForkId, fork: SharedBackend) -> Self {
        let launched_with_fork = Some(id.clone());
        let mut backend = Self { launched_with_fork, ..Default::default() };
        backend.insert_new_fork(id, fork);
        backend
    }

    pub fn ensure_fork_id(&self, id: U256) -> eyre::Result<&ForkId> {
        self.issued_local_fork_ids
            .get(&id)
            .ok_or_else(|| eyre::eyre!("No matching fork found for {}", id))
    }

    /// Ensures that the `SharedBackend` for the given `id` exits
    pub fn ensure_backend(&self, id: U256) -> eyre::Result<&SharedBackend> {
        self.get_backend(id).ok_or_else(|| eyre::eyre!("No matching fork found for {}", id))
    }

    /// Returns the matching `SharedBackend` for the givne id
    fn get_backend(&self, id: U256) -> Option<&SharedBackend> {
        self.issued_local_fork_ids.get(&id).and_then(|id| self.created_forks.get(id))
    }

    /// Updates the fork and the local mapping
    pub fn update_fork_mapping(&mut self, id: U256, fork: ForkId, backend: SharedBackend) {
        self.created_forks.insert(fork.clone(), backend);
        self.issued_local_fork_ids.insert(id, fork);
    }

    /// Issues a new local fork identifier
    pub fn insert_new_fork(&mut self, fork: ForkId, backend: SharedBackend) -> U256 {
        self.created_forks.insert(fork.clone(), backend);
        let id = self.next_id();
        self.issued_local_fork_ids.insert(id, fork);
        id
    }

    fn next_id(&mut self) -> U256 {
        let id = self.next_fork_id;
        self.next_fork_id += U256::one();
        id
    }
}

/// This updates the currently used env with the fork's environment
pub(crate) fn update_current_env_with_fork_env(current: &mut Env, fork: Env) {
    current.block = fork.block;
    current.cfg = fork.cfg;
}
