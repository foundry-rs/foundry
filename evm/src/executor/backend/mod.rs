use crate::{
    abi::CHEATCODE_ADDRESS,
    executor::{
        backend::snapshot::BackendSnapshot,
        fork::{CreateFork, ForkId, MultiFork, SharedBackend},
        inspector::DEFAULT_CREATE2_DEPLOYER,
        snapshot::Snapshots,
    },
};
use ethers::{
    prelude::{H160, H256, U256},
    types::Address,
};
use hashbrown::HashMap as Map;
pub use in_memory_db::MemDb;
use revm::{
    db::{CacheDB, DatabaseRef},
    Account, AccountInfo, Bytecode, Database, DatabaseCommit, Env, ExecutionResult, InMemoryDB,
    Inspector, JournaledState, Log, TransactTo, KECCAK_EMPTY,
};
use std::collections::{HashMap, HashSet};
use tracing::{trace, warn};

mod fuzz;
pub mod snapshot;
pub use fuzz::FuzzBackendWrapper;
mod diagnostic;
use crate::executor::inspector::cheatcodes::util::with_journaled_account;
pub use diagnostic::RevertDiagnostic;

mod in_memory_db;

// A `revm::Database` that is used in forking mode
type ForkDB = CacheDB<SharedBackend>;

/// Represents a numeric `ForkId` valid only for the existence of the `Backend`.
/// The difference between `ForkId` and `LocalForkId` is that `ForkId` tracks pairs of `endpoint +
/// block` which can be reused by multiple tests, whereas the `LocalForkId` is unique within a test
pub type LocalForkId = U256;

/// Represents the index of a fork in the created forks vector
/// This is used for fast lookup
type ForkLookupIndex = usize;

/// All accounts that will have persistent storage across fork swaps. See also [`clone_data()`]
const DEFAULT_PERSISTENT_ACCOUNTS: [H160; 2] = [CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER];

/// An extension trait that allows us to easily extend the `revm::Inspector` capabilities
#[auto_impl::auto_impl(&mut, Box)]
pub trait DatabaseExt: Database {
    /// Creates a new snapshot at the current point of execution.
    ///
    /// A snapshot is associated with a new unique id that's created for the snapshot.
    /// Snapshots can be reverted: [DatabaseExt::revert], however a snapshot can only be reverted
    /// once. After a successful revert, the same snapshot id cannot be used again.
    fn snapshot(&mut self, journaled_state: &JournaledState, env: &Env) -> U256;
    /// Reverts the snapshot if it exists
    ///
    /// Returns `true` if the snapshot was successfully reverted, `false` if no snapshot for that id
    /// exists.
    ///
    /// **N.B.** While this reverts the state of the evm to the snapshot, it keeps new logs made
    /// since the snapshots was created. This way we can show logs that were emitted between
    /// snapshot and its revert.
    /// This will also revert any changes in the `Env` and replace it with the captured `Env` of
    /// `Self::snapshot`
    fn revert(
        &mut self,
        id: U256,
        journaled_state: &JournaledState,
        env: &mut Env,
    ) -> Option<JournaledState>;

    /// Creates and also selects a new fork
    ///
    /// This is basically `create_fork` + `select_fork`
    fn create_select_fork(
        &mut self,
        fork: CreateFork,
        env: &mut Env,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<LocalForkId> {
        let id = self.create_fork(fork, journaled_state)?;
        self.select_fork(id, env, journaled_state)?;
        Ok(id)
    }

    /// Creates a new fork but does _not_ select it
    fn create_fork(
        &mut self,
        fork: CreateFork,
        journaled_state: &JournaledState,
    ) -> eyre::Result<LocalForkId>;

    /// Selects the fork's state
    ///
    /// This will also modify the current `Env`.
    ///
    /// **Note**: this does not change the local state, but swaps the remote state
    ///
    /// # Errors
    ///
    /// Returns an error if no fork with the given `id` exists
    fn select_fork(
        &mut self,
        id: LocalForkId,
        env: &mut Env,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<()>;

    /// Updates the fork to given block number.
    ///
    /// This will essentially create a new fork at the given block height.
    ///
    /// # Errors
    ///
    /// Returns an error if not matching fork was found.
    fn roll_fork(
        &mut self,
        id: Option<LocalForkId>,
        block_number: U256,
        env: &mut Env,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<()>;

    /// Returns the `ForkId` that's currently used in the database, if fork mode is on
    fn active_fork_id(&self) -> Option<LocalForkId>;

    /// Whether the database is currently in forked
    fn is_forked_mode(&self) -> bool {
        self.active_fork_id().is_some()
    }

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
    fn ensure_fork(&self, id: Option<LocalForkId>) -> eyre::Result<LocalForkId>;

    /// Ensures that a corresponding `ForkId` exists for the given local `id`
    fn ensure_fork_id(&self, id: LocalForkId) -> eyre::Result<&ForkId>;

    /// Handling multiple accounts/new contracts in a multifork environment can be challenging since
    /// every fork has its own standalone storage section. So this can be a common error to run
    /// into:
    ///
    /// ```solidity
    /// function testCanDeploy() public {
    ///    cheats.selectFork(mainnetFork);
    ///    // contract created while on `mainnetFork`
    ///    DummyContract dummy = new DummyContract();
    ///    // this will succeed
    ///    dummy.hello();
    ///
    ///    cheats.selectFork(optimismFork);
    ///
    ///    cheats.expectRevert();
    ///    // this will revert since `dummy` contract only exists on `mainnetFork`
    ///    dummy.hello();
    /// }
    /// ```
    ///
    /// If this happens (`dummy.hello()`), or more general, a call on an address that's not a
    /// contract, revm will revert without useful context. This call will check in this context if
    /// `address(dummy)` belongs to an existing contract and if not will check all other forks if
    /// the contract is deployed there.
    ///
    /// Returns a more useful error message if that's the case
    fn diagnose_revert(
        &self,
        callee: Address,
        journaled_state: &JournaledState,
    ) -> Option<RevertDiagnostic>;

    /// Returns true if the given account is currently marked as persistent.
    fn is_persistent(&self, acc: &Address) -> bool;

    /// Revokes persistent status from the given account.
    fn remove_persistent_account(&mut self, account: &Address) -> bool;

    /// Marks the given account as persistent.
    fn add_persistent_account(&mut self, account: Address) -> bool;

    /// Removes persistent status from all given accounts
    fn remove_persistent_accounts(&mut self, accounts: impl IntoIterator<Item = Address>) {
        for acc in accounts {
            self.remove_persistent_account(&acc);
        }
    }

    /// Extends the persistent accounts with the accounts the iterator yields.
    fn extend_persistent_accounts(&mut self, accounts: impl IntoIterator<Item = Address>) {
        for acc in accounts {
            self.add_persistent_account(acc);
        }
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
/// Each `Fork`, identified by a unique id, uses completely separate storage, write operations are
/// performed only in the fork's own database, `ForkDB`.
///
/// A `ForkDB` consists of 2 halves:
///   - everything fetched from the remote is readonly
///   - all local changes (instructed by the contract) are written to the backend's `db` and don't
///     alter the state of the remote client.
///
/// # Fork swapping
///
/// Multiple "forks" can be created `Backend::create_fork()`, however only 1 can be used by the
/// `db`. However, their state can be hot-swapped by swapping the read half of `db` from one fork to
/// another.
/// When swapping forks (`Backend::select_fork()`) we also update the current `Env` of the `EVM`
/// accordingly, so that all `block.*` config values match
///
/// When another for is selected [`DatabaseExt::select_fork()`] the entire storage, including
/// `JournaledState` is swapped, but the storage of the caller's and the test contract account is
/// _always_ cloned. This way a fork has entirely separate storage but data can still be shared
/// across fork boundaries via stack and contract variables.
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
    // The default in memory db
    mem_db: InMemoryDB,
    /// The journaled_state to use to initialize new forks with
    ///
    /// The way [`revm::JournaledState`] works is, that it holds the "hot" accounts loaded from the
    /// underlying `Database` that feeds the Account and State data ([`revm::AccountInfo`])to the
    /// journaled_state so it can apply changes to the state while the evm executes.
    ///
    /// In a way the `JournaledState` is something like a cache that
    /// 1. check if account is already loaded (hot)
    /// 2. if not load from the `Database` (this will then retrieve the account via RPC in forking
    /// mode)
    ///
    /// To properly initialize we store the `JournaledState` before the first fork is selected
    /// ([`DatabaseExt::select_fork`]).
    ///
    /// This will be an empty `JournaledState`, which will be populated with persistent accounts,
    /// See [`Self::update_fork_db()`] and [`clone_data()`].
    fork_init_journaled_state: JournaledState,
    /// The currently active fork database
    ///
    /// If this is set, then the Backend is currently in forking mode
    active_fork_ids: Option<(LocalForkId, ForkLookupIndex)>,
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
        // Note: this will take of registering the `fork`
        let mut backend = Self {
            forks,
            mem_db: InMemoryDB::default(),
            fork_init_journaled_state: Default::default(),
            active_fork_ids: None,
            inner: BackendInner {
                persistent_accounts: HashSet::from(DEFAULT_PERSISTENT_ACCOUNTS),
                ..Default::default()
            },
        };

        if let Some(fork) = fork {
            let (fork_id, fork, _) =
                backend.forks.create_fork(fork).expect("Unable to create fork");
            let fork_db = ForkDB::new(fork);
            let fork_ids =
                backend.inner.insert_new_fork(fork_id.clone(), fork_db, Default::default());
            backend.inner.launched_with_fork = Some((fork_id, fork_ids.0, fork_ids.1));
            backend.active_fork_ids = Some(fork_ids);
        }

        backend
    }

    /// Creates a new instance with a `BackendDatabase::InMemory` cache layer for the `CacheDB`
    pub fn clone_empty(&self) -> Self {
        Self {
            forks: self.forks.clone(),
            mem_db: InMemoryDB::default(),
            fork_init_journaled_state: Default::default(),
            active_fork_ids: None,
            inner: Default::default(),
        }
    }

    pub fn insert_account_info(&mut self, address: H160, account: AccountInfo) {
        if let Some(db) = self.active_fork_db_mut() {
            db.insert_account_info(address, account)
        } else {
            self.mem_db.insert_account_info(address, account)
        }
    }

    /// Returns all snapshots created in this backend
    pub fn snapshots(&self) -> &Snapshots<BackendSnapshot<BackendDatabaseSnapshot>> {
        &self.inner.snapshots
    }

    /// Sets the address of the `DSTest` contract that is being executed
    ///
    /// This will also mark the caller as persistent and remove the persistent status from the
    /// previous test contract address
    pub fn set_test_contract(&mut self, acc: Address) -> &mut Self {
        trace!(?acc, "setting test account");
        // toggle the previous sender
        if let Some(current) = self.inner.test_contract_address.take() {
            self.remove_persistent_account(&current);
        }

        self.add_persistent_account(acc);
        self.inner.test_contract_address = Some(acc);
        self
    }

    /// Sets the caller address
    pub fn set_caller(&mut self, acc: Address) -> &mut Self {
        trace!(?acc, "setting caller account");
        self.inner.caller = Some(acc);
        self
    }

    /// Returns the address of the set `DSTest` contract
    pub fn test_contract_address(&self) -> Option<Address> {
        self.inner.test_contract_address
    }

    /// Returns the set caller address
    pub fn caller_address(&self) -> Option<Address> {
        self.inner.caller
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

    /// when creating or switching forks, we update the AccountInfo of the contract
    pub(crate) fn update_fork_db(&self, journaled_state: &mut JournaledState, fork: &mut Fork) {
        debug_assert!(
            self.inner.test_contract_address.is_some(),
            "Test contract address must be set"
        );

        self.update_fork_db_contracts(
            self.inner.persistent_accounts.iter().copied(),
            journaled_state,
            fork,
        )
    }

    /// Copies the state of all `accounts` from the currently active db into the given `fork`
    pub(crate) fn update_fork_db_contracts(
        &self,
        accounts: impl IntoIterator<Item = Address>,
        journaled_state: &mut JournaledState,
        fork: &mut Fork,
    ) {
        if let Some((_, fork_idx)) = self.active_fork_ids.as_ref() {
            let active = self.inner.get_fork(*fork_idx);
            clone_data(accounts, &active.db, journaled_state, fork)
        } else {
            clone_data(accounts, &self.mem_db, journaled_state, fork)
        }
    }

    /// Returns the memory db used if not in forking mode
    pub fn mem_db(&self) -> &InMemoryDB {
        &self.mem_db
    }

    /// Returns true if the `id` is currently active
    pub fn is_active_fork(&self, id: LocalForkId) -> bool {
        self.active_fork_ids.map(|(i, _)| i == id).unwrap_or_default()
    }

    /// Returns `true` if the `Backend` is currently in forking mode
    pub fn is_in_forking_mode(&self) -> bool {
        self.active_fork().is_some()
    }

    /// Returns the currently active `Fork`, if any
    pub fn active_fork(&self) -> Option<&Fork> {
        self.active_fork_ids.map(|(_, idx)| self.inner.get_fork(idx))
    }

    /// Returns the currently active `Fork`, if any
    pub fn active_fork_mut(&mut self) -> Option<&mut Fork> {
        self.active_fork_ids.map(|(_, idx)| self.inner.get_fork_mut(idx))
    }

    /// Returns the currently active `ForkDB`, if any
    pub fn active_fork_db(&self) -> Option<&ForkDB> {
        self.active_fork().map(|f| &f.db)
    }

    /// Returns the currently active `ForkDB`, if any
    fn active_fork_db_mut(&mut self) -> Option<&mut ForkDB> {
        self.active_fork_mut().map(|f| &mut f.db)
    }

    /// Creates a snapshot of the currently active database
    pub(crate) fn create_db_snapshot(&self) -> BackendDatabaseSnapshot {
        if let Some((id, idx)) = self.active_fork_ids {
            let fork = self.inner.get_fork(idx).clone();
            let fork_id = self.inner.ensure_fork_id(id).cloned().expect("Exists; qed");
            BackendDatabaseSnapshot::Forked(id, fork_id, idx, Box::new(fork))
        } else {
            BackendDatabaseSnapshot::InMemory(self.mem_db.clone())
        }
    }

    /// Since each `Fork` tracks logs separately, we need to merge them to get _all_ of them
    pub fn merged_logs(&self, mut logs: Vec<Log>) -> Vec<Log> {
        if let Some((_, active)) = self.active_fork_ids {
            let mut all_logs = Vec::with_capacity(logs.len());

            self.inner
                .forks
                .iter()
                .enumerate()
                .filter_map(|(idx, f)| f.as_ref().map(|f| (idx, f)))
                .for_each(|(idx, f)| {
                    if idx == active {
                        all_logs.append(&mut logs);
                    } else {
                        all_logs.extend(f.journaled_state.logs.clone())
                    }
                });
            return all_logs
        }

        logs
    }

    /// Executes the configured test call of the `env` without commiting state changes
    pub fn inspect_ref<INSP>(
        &mut self,
        mut env: Env,
        mut inspector: INSP,
    ) -> (ExecutionResult, Map<Address, Account>)
    where
        INSP: Inspector<Self>,
    {
        self.set_caller(env.tx.caller);
        if let TransactTo::Call(to) = env.tx.transact_to {
            self.set_test_contract(to);
        }
        revm::evm_inner::<Self, true>(&mut env, self, &mut inspector).transact()
    }

    /// Ths will clean up already loaded accounts that would be initialized without the correct data
    /// from the fork
    ///
    /// It can happen that an account is loaded before the first fork is selected, like
    /// `getNonce(addr)`, which will load an empty account by default.
    ///
    /// This account data then would not match the account data of a fork if it exists.
    /// So when the first fork is initialized we replace these accounts with the actual account as
    /// it exists on the fork.
    fn prepare_init_journal_state(&mut self) {
        let loaded_accounts = self
            .fork_init_journaled_state
            .state
            .iter()
            .filter(|(addr, acc)| {
                !acc.is_existing_precompile && acc.is_touched && !self.is_persistent(addr)
            })
            .map(|(addr, _)| addr)
            .copied()
            .collect::<Vec<_>>();

        for fork in self.inner.forks_iter_mut() {
            let mut journaled_state = self.fork_init_journaled_state.clone();
            for loaded_account in loaded_accounts.iter().copied() {
                trace!(?loaded_account, "replacing account on init");
                let fork_account = fork.db.basic(loaded_account);
                let init_account =
                    journaled_state.state.get_mut(&loaded_account).expect("exists; qed");
                init_account.info = fork_account;
            }
            fork.journaled_state = journaled_state;
        }
    }
}

// === impl a bunch of `revm::Database` adjacent implementations ===

impl DatabaseExt for Backend {
    fn snapshot(&mut self, journaled_state: &JournaledState, env: &Env) -> U256 {
        trace!("create snapshot");
        let id = self.inner.snapshots.insert(BackendSnapshot::new(
            self.create_db_snapshot(),
            journaled_state.clone(),
            env.clone(),
        ));
        trace!(target: "backend", "Created new snapshot {}", id);
        id
    }

    fn revert(
        &mut self,
        id: U256,
        current_state: &JournaledState,
        current: &mut Env,
    ) -> Option<JournaledState> {
        trace!(?id, "revert snapshot");
        if let Some(mut snapshot) = self.inner.snapshots.remove(id) {
            // need to check whether DSTest's `failed` variable is set to `true` which means an
            // error occurred either during the snapshot or even before
            if self.is_failed() {
                self.inner.has_failure_snapshot = true;
            }

            // merge additional logs
            snapshot.merge(current_state);
            let BackendSnapshot { db, mut journaled_state, env } = snapshot;
            match db {
                BackendDatabaseSnapshot::InMemory(mem_db) => {
                    self.mem_db = mem_db;
                }
                BackendDatabaseSnapshot::Forked(id, fork_id, idx, mut fork) => {
                    // there might be the case where the snapshot was created during `setUp` with
                    // another caller, so we need to ensure the caller account is present in the
                    // journaled state and database
                    let caller = current.tx.caller;
                    if !journaled_state.state.contains_key(&caller) {
                        let caller_account = current_state
                            .state
                            .get(&caller)
                            .map(|acc| acc.info.clone())
                            .unwrap_or_default();

                        if !fork.db.accounts.contains_key(&caller) {
                            // update the caller account which is required by the evm
                            fork.db.insert_account_info(caller, caller_account.clone());
                            with_journaled_account(
                                &mut fork.journaled_state,
                                &mut fork.db,
                                caller,
                                |_| (),
                            );
                        }
                        journaled_state.state.insert(caller, caller_account.into());
                    }
                    self.inner.revert_snapshot(id, fork_id, idx, *fork);
                    self.active_fork_ids = Some((id, idx))
                }
            }

            update_current_env_with_fork_env(current, env);
            trace!(target: "backend", "Reverted snapshot {}", id);

            Some(journaled_state)
        } else {
            warn!(target: "backend", "No snapshot to revert for {}", id);
            None
        }
    }

    fn create_fork(
        &mut self,
        fork: CreateFork,
        journaled_state: &JournaledState,
    ) -> eyre::Result<LocalForkId> {
        trace!("create fork");
        let (fork_id, fork, _) = self.forks.create_fork(fork)?;
        let fork_db = ForkDB::new(fork);

        // there might be the case where a fork was previously created and selected during `setUp`
        // not necessarily with the current caller in which case we need to ensure that the init
        // state also includes the caller
        if let Some(caller) = self.caller_address() {
            if !self.fork_init_journaled_state.state.contains_key(&caller) {
                if let Some(account) = journaled_state.state.get(&caller).cloned() {
                    self.fork_init_journaled_state.state.insert(caller, account);
                }
            }
        }

        let (id, _) =
            self.inner.insert_new_fork(fork_id, fork_db, self.fork_init_journaled_state.clone());
        Ok(id)
    }

    /// When switching forks we copy the shared state
    fn select_fork(
        &mut self,
        id: LocalForkId,
        env: &mut Env,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<()> {
        trace!(?id, "select fork");
        if self.is_active_fork(id) {
            // nothing to do
            return Ok(())
        }

        let fork_id = self.ensure_fork_id(id).cloned()?;
        let idx = self.inner.ensure_fork_index(&fork_id)?;
        let fork_env = self
            .forks
            .get_env(fork_id)?
            .ok_or_else(|| eyre::eyre!("Requested fork `{}` does not exit", id))?;

        let launched_with_fork = self.inner.launched_with_fork.is_some();

        // If we're currently in forking mode we need to update the journaled_state to this point,
        // this ensures the changes performed while the fork was active are recorded
        if let Some(active) = self.active_fork_mut() {
            active.journaled_state = journaled_state.clone();

            // if the Backend was launched in forking mode, then we also need to adjust the depth of
            // the `JournalState` at this point
            if launched_with_fork {
                let caller = env.tx.caller;
                let caller_account =
                    active.journaled_state.state.get(&env.tx.caller).map(|acc| acc.info.clone());
                let target_fork = self.inner.get_fork_mut(idx);
                if target_fork.journaled_state.depth == 0 {
                    // depth 0 will be the default value when the fork was created and since we
                    // launched in forking mode there never is a `depth` that can be set for the
                    // `fork_init_journaled_state` instead we need to manually bump the depth to the
                    // current depth of the call _once_
                    target_fork.journaled_state.depth = journaled_state.depth;

                    // we also need to initialize and touch the caller
                    if let Some(acc) = caller_account {
                        target_fork.db.insert_account_info(caller, acc);
                        with_journaled_account(
                            &mut target_fork.journaled_state,
                            &mut target_fork.db,
                            caller,
                            |_| (),
                        );
                    }
                }
            }
        } else {
            // this is the first time a fork is selected. This means up to this point all changes
            // are made in a single `JournaledState`, for example after a `setup` that only created
            // different forks. Since the `JournaledState` is valid for all forks until the
            // first fork is selected, we need to update it for all forks and use it as init state
            // for all future forks
            trace!("recording fork init journaled_state");
            self.fork_init_journaled_state = journaled_state.clone();
            self.prepare_init_journal_state();
        }

        // update the shared state and track
        let mut fork = self.inner.take_fork(idx);
        self.update_fork_db(journaled_state, &mut fork);
        self.inner.set_fork(idx, fork);

        self.active_fork_ids = Some((id, idx));
        // update the environment accordingly
        update_current_env_with_fork_env(env, fork_env);
        Ok(())
    }

    /// This is effectively the same as [`Self::create_select_fork()`] but updating an existing fork
    fn roll_fork(
        &mut self,
        id: Option<LocalForkId>,
        block_number: U256,
        env: &mut Env,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<()> {
        trace!(?id, ?block_number, "roll fork");
        let id = self.ensure_fork(id)?;
        let (fork_id, backend, fork_env) =
            self.forks.roll_fork(self.inner.ensure_fork_id(id).cloned()?, block_number.as_u64())?;
        // this will update the local mapping
        self.inner.roll_fork(id, fork_id, backend)?;

        if let Some((active_id, active_idx)) = self.active_fork_ids {
            // the currently active fork is the targeted fork of this call
            if active_id == id {
                // need to update the block's env settings right away, which is otherwise set when
                // forks are selected `select_fork`
                update_current_env_with_fork_env(env, fork_env);

                // we also need to update the journaled_state right away, this has essentially the
                // same effect as selecting (`select_fork`) by discarding
                // non-persistent storage from the journaled_state. This which will
                // reset cached state from the previous block
                let persitent_addrs = self.inner.persistent_accounts.clone();
                let active = self.inner.get_fork_mut(active_idx);
                active.journaled_state = self.fork_init_journaled_state.clone();
                for addr in persitent_addrs {
                    clone_journaled_state_data(addr, journaled_state, &mut active.journaled_state);
                }
                *journaled_state = active.journaled_state.clone();
            }
        }
        Ok(())
    }

    fn active_fork_id(&self) -> Option<LocalForkId> {
        self.active_fork_ids.map(|(id, _)| id)
    }

    fn ensure_fork(&self, id: Option<LocalForkId>) -> eyre::Result<LocalForkId> {
        if let Some(id) = id {
            if self.inner.issued_local_fork_ids.contains_key(&id) {
                return Ok(id)
            }
            eyre::bail!("Requested fork `{}` does not exit", id)
        }
        if let Some(id) = self.active_fork_id() {
            Ok(id)
        } else {
            eyre::bail!("No fork active")
        }
    }

    fn ensure_fork_id(&self, id: LocalForkId) -> eyre::Result<&ForkId> {
        self.inner.ensure_fork_id(id)
    }

    fn diagnose_revert(
        &self,
        callee: Address,
        _journaled_state: &JournaledState,
    ) -> Option<RevertDiagnostic> {
        let active_id = self.active_fork_id()?;
        let active_fork = self.active_fork()?;
        if !active_fork.is_contract(callee) {
            // no contract for `callee` available on current fork, check if available on other forks
            let mut available_on = Vec::new();
            for (id, fork) in self.inner.forks_iter().filter(|(id, _)| *id != active_id) {
                if fork.is_contract(callee) {
                    available_on.push(id);
                }
            }

            return if available_on.is_empty() {
                Some(RevertDiagnostic::ContractDoesNotExist { contract: callee, active: active_id })
            } else {
                // likely user error: called a contract that's not available on active fork but is
                // present other forks
                Some(RevertDiagnostic::ContractExistsOnOtherForks {
                    contract: callee,
                    active: active_id,
                    available_on,
                })
            }
        }
        None
    }

    fn is_persistent(&self, acc: &Address) -> bool {
        self.inner.persistent_accounts.contains(acc)
    }

    fn remove_persistent_account(&mut self, account: &Address) -> bool {
        trace!(?account, "remove persistent account");
        self.inner.persistent_accounts.remove(account)
    }

    fn add_persistent_account(&mut self, account: Address) -> bool {
        trace!(?account, "add persistent account");
        self.inner.persistent_accounts.insert(account)
    }
}

impl DatabaseRef for Backend {
    fn basic(&self, address: H160) -> AccountInfo {
        if let Some(db) = self.active_fork_db() {
            db.basic(address)
        } else {
            self.mem_db.basic(address)
        }
    }

    fn code_by_hash(&self, code_hash: H256) -> Bytecode {
        if let Some(db) = self.active_fork_db() {
            db.code_by_hash(code_hash)
        } else {
            self.mem_db.code_by_hash(code_hash)
        }
    }

    fn storage(&self, address: H160, index: U256) -> U256 {
        if let Some(db) = self.active_fork_db() {
            DatabaseRef::storage(db, address, index)
        } else {
            DatabaseRef::storage(&self.mem_db, address, index)
        }
    }

    fn block_hash(&self, number: U256) -> H256 {
        if let Some(db) = self.active_fork_db() {
            db.block_hash(number)
        } else {
            self.mem_db.block_hash(number)
        }
    }
}

impl<'a> DatabaseRef for &'a mut Backend {
    fn basic(&self, address: H160) -> AccountInfo {
        if let Some(db) = self.active_fork_db() {
            DatabaseRef::basic(db, address)
        } else {
            DatabaseRef::basic(&self.mem_db, address)
        }
    }

    fn code_by_hash(&self, code_hash: H256) -> Bytecode {
        if let Some(db) = self.active_fork_db() {
            DatabaseRef::code_by_hash(db, code_hash)
        } else {
            DatabaseRef::code_by_hash(&self.mem_db, code_hash)
        }
    }

    fn storage(&self, address: H160, index: U256) -> U256 {
        if let Some(db) = self.active_fork_db() {
            DatabaseRef::storage(db, address, index)
        } else {
            DatabaseRef::storage(&self.mem_db, address, index)
        }
    }

    fn block_hash(&self, number: U256) -> H256 {
        if let Some(db) = self.active_fork_db() {
            DatabaseRef::block_hash(db, number)
        } else {
            DatabaseRef::block_hash(&self.mem_db, number)
        }
    }
}

impl DatabaseCommit for Backend {
    fn commit(&mut self, changes: Map<H160, Account>) {
        if let Some(db) = self.active_fork_db_mut() {
            db.commit(changes)
        } else {
            self.mem_db.commit(changes)
        }
    }
}

impl Database for Backend {
    fn basic(&mut self, address: H160) -> AccountInfo {
        if let Some(db) = self.active_fork_db_mut() {
            db.basic(address)
        } else {
            self.mem_db.basic(address)
        }
    }

    fn code_by_hash(&mut self, code_hash: H256) -> Bytecode {
        if let Some(db) = self.active_fork_db_mut() {
            db.code_by_hash(code_hash)
        } else {
            self.mem_db.code_by_hash(code_hash)
        }
    }

    fn storage(&mut self, address: H160, index: U256) -> U256 {
        if let Some(db) = self.active_fork_db_mut() {
            Database::storage(db, address, index)
        } else {
            Database::storage(&mut self.mem_db, address, index)
        }
    }

    fn block_hash(&mut self, number: U256) -> H256 {
        if let Some(db) = self.active_fork_db_mut() {
            db.block_hash(number)
        } else {
            self.mem_db.block_hash(number)
        }
    }
}

/// Variants of a [revm::Database]
#[derive(Debug, Clone)]
pub enum BackendDatabaseSnapshot {
    /// Simple in-memory [revm::Database]
    InMemory(InMemoryDB),
    /// Contains the entire forking mode database
    Forked(LocalForkId, ForkId, ForkLookupIndex, Box<Fork>),
}

/// Represents a fork
#[derive(Debug, Clone)]
pub struct Fork {
    db: ForkDB,
    journaled_state: JournaledState,
}

// === impl Fork ===

impl Fork {
    /// Returns true if the account is a contract
    pub fn is_contract(&self, acc: Address) -> bool {
        self.db.basic(acc).code_hash != KECCAK_EMPTY ||
            self.journaled_state
                .state
                .get(&acc)
                .map(|acc| acc.info.code_hash != KECCAK_EMPTY)
                .unwrap_or_default()
    }
}

/// Container type for various Backend related data
#[derive(Debug, Clone, Default)]
pub struct BackendInner {
    /// Stores the `ForkId` of the fork the `Backend` launched with from the start.
    ///
    /// In other words if [`Backend::spawn()`] was called with a `CreateFork` command, to launch
    /// directly in fork mode, this holds the corresponding fork identifier of this fork.
    pub launched_with_fork: Option<(ForkId, LocalForkId, ForkLookupIndex)>,
    /// This tracks numeric fork ids and the `ForkId` used by the handler.
    ///
    /// This is necessary, because there can be multiple `Backends` associated with a single
    /// `ForkId` which is only a pair of endpoint + block. Since an existing fork can be
    /// modified (e.g. `roll_fork`), but this should only affect the fork that's unique for the
    /// test and not the `ForkId`
    ///
    /// This ensures we can treat forks as unique from the context of a test, so rolling to another
    /// is basically creating(or reusing) another `ForkId` that's then mapped to the previous
    /// issued _local_ numeric identifier, that remains constant, even if the underlying fork
    /// backend changes.
    pub issued_local_fork_ids: HashMap<LocalForkId, ForkId>,
    /// tracks all the created forks
    /// Contains the index of the corresponding `ForkDB` in the `forks` vec
    pub created_forks: HashMap<ForkId, ForkLookupIndex>,
    /// Holds all created fork databases
    // Note: data is stored in an `Option` so we can remove it without reshuffling
    pub forks: Vec<Option<Fork>>,
    /// Contains snapshots made at a certain point
    pub snapshots: Snapshots<BackendSnapshot<BackendDatabaseSnapshot>>,
    /// Tracks whether there was a failure in a snapshot that was reverted
    ///
    /// The Test contract contains a bool variable that is set to true when an `assert` function
    /// failed. When a snapshot is reverted, it reverts the state of the evm, but we still want
    /// to know if there was an `assert` that failed after the snapshot was taken so that we can
    /// check if the test function passed all asserts even across snapshots. When a snapshot is
    /// reverted we get the _current_ `revm::JournaledState` which contains the state that we can
    /// check if the `_failed` variable is set,
    /// additionally
    pub has_failure_snapshot: bool,
    /// Tracks the address of a Test contract
    ///
    /// This address can be used to inspect the state of the contract when a test is being
    /// executed. E.g. the `_failed` variable of `DSTest`
    pub test_contract_address: Option<Address>,
    /// Tracks the caller of the test function
    pub caller: Option<Address>,
    /// Tracks numeric identifiers for forks
    pub next_fork_id: LocalForkId,
    /// All accounts that should be kept persistent when switching forks.
    /// This means all accounts stored here _don't_ use a separate storage section on each fork
    /// instead the use only one that's persistent across fork swaps.
    ///
    /// See also [`clone_data()`]
    pub persistent_accounts: HashSet<Address>,
}

// === impl BackendInner ===

impl BackendInner {
    pub fn ensure_fork_id(&self, id: LocalForkId) -> eyre::Result<&ForkId> {
        self.issued_local_fork_ids
            .get(&id)
            .ok_or_else(|| eyre::eyre!("No matching fork found for {}", id))
    }

    pub fn ensure_fork_index(&self, id: &ForkId) -> eyre::Result<ForkLookupIndex> {
        self.created_forks
            .get(id)
            .copied()
            .ok_or_else(|| eyre::eyre!("No matching fork found for {}", id))
    }

    /// Returns the underlying
    #[track_caller]
    fn get_fork(&self, idx: ForkLookupIndex) -> &Fork {
        debug_assert!(idx < self.forks.len(), "fork lookup index must exist");
        self.forks[idx].as_ref().unwrap()
    }

    /// Returns the underlying
    #[track_caller]
    fn get_fork_mut(&mut self, idx: ForkLookupIndex) -> &mut Fork {
        debug_assert!(idx < self.forks.len(), "fork lookup index must exist");
        self.forks[idx].as_mut().unwrap()
    }

    /// Removes the fork
    fn take_fork(&mut self, idx: ForkLookupIndex) -> Fork {
        debug_assert!(idx < self.forks.len(), "fork lookup index must exist");
        self.forks[idx].take().unwrap()
    }

    fn set_fork(&mut self, idx: ForkLookupIndex, fork: Fork) {
        self.forks[idx] = Some(fork)
    }

    /// Returns an iterator over Forks
    pub fn forks_iter(&self) -> impl Iterator<Item = (LocalForkId, &Fork)> + '_ {
        self.issued_local_fork_ids
            .iter()
            .map(|(id, fork_id)| (*id, self.get_fork(self.created_forks[fork_id])))
    }

    /// Returns a mutable iterator over all Forks
    pub fn forks_iter_mut(&mut self) -> impl Iterator<Item = &mut Fork> + '_ {
        self.forks.iter_mut().filter_map(|f| f.as_mut())
    }

    /// Reverts the entire fork database
    pub fn revert_snapshot(
        &mut self,
        id: LocalForkId,
        fork_id: ForkId,
        idx: ForkLookupIndex,
        fork: Fork,
    ) {
        self.created_forks.insert(fork_id.clone(), idx);
        self.issued_local_fork_ids.insert(id, fork_id);
        self.set_fork(idx, fork)
    }

    /// Updates the fork and the local mapping and returns the new index for the `fork_db`
    pub fn update_fork_mapping(
        &mut self,
        id: LocalForkId,
        fork_id: ForkId,
        db: ForkDB,
        journaled_state: JournaledState,
    ) -> ForkLookupIndex {
        let idx = self.forks.len();
        self.issued_local_fork_ids.insert(id, fork_id.clone());
        self.created_forks.insert(fork_id, idx);

        let fork = Fork { db, journaled_state };
        self.forks.push(Some(fork));
        idx
    }

    pub fn roll_fork(
        &mut self,
        id: LocalForkId,
        new_fork_id: ForkId,
        backend: SharedBackend,
    ) -> eyre::Result<ForkLookupIndex> {
        let fork_id = self.ensure_fork_id(id)?;
        let idx = self.ensure_fork_index(fork_id)?;

        if let Some(active) = self.forks[idx].as_mut() {
            // we initialize a _new_ `ForkDB` but keep the state of persistent accounts
            let mut new_db = ForkDB::new(backend);
            for addr in self.persistent_accounts.iter().copied() {
                clone_db_account_data(addr, &active.db, &mut new_db);
            }
            active.db = new_db;
        }
        // update mappings
        self.issued_local_fork_ids.insert(id, new_fork_id.clone());
        self.created_forks.insert(new_fork_id, idx);
        Ok(idx)
    }

    /// Inserts a _new_ `ForkDB` and issues a new local fork identifier
    ///
    /// Also returns the index where the `ForDB` is stored
    pub fn insert_new_fork(
        &mut self,
        fork_id: ForkId,
        db: ForkDB,
        journaled_state: JournaledState,
    ) -> (LocalForkId, ForkLookupIndex) {
        let idx = self.forks.len();
        self.created_forks.insert(fork_id.clone(), idx);
        let id = self.next_id();
        self.issued_local_fork_ids.insert(id, fork_id);
        let fork = Fork { db, journaled_state };
        self.forks.push(Some(fork));
        (id, idx)
    }

    fn next_id(&mut self) -> U256 {
        let id = self.next_fork_id;
        self.next_fork_id += U256::one();
        id
    }

    /// Returns the number of issued ids
    pub fn len(&self) -> usize {
        self.issued_local_fork_ids.len()
    }

    /// Returns true if no forks are issued
    pub fn is_empty(&self) -> bool {
        self.issued_local_fork_ids.is_empty()
    }
}

/// This updates the currently used env with the fork's environment
pub(crate) fn update_current_env_with_fork_env(current: &mut Env, fork: Env) {
    current.block = fork.block;
    current.cfg = fork.cfg;
}

/// Clones the data of the given `accounts` from the `active` database into the `fork_db`
/// This includes the data held in storage (`CacheDB`) and kept in the `JournaledState`
pub(crate) fn clone_data<ExtDB: DatabaseRef>(
    accounts: impl IntoIterator<Item = Address>,
    active: &CacheDB<ExtDB>,
    active_journaled_state: &mut JournaledState,
    fork: &mut Fork,
) {
    for addr in accounts.into_iter() {
        clone_db_account_data(addr, active, &mut fork.db);
        clone_journaled_state_data(addr, active_journaled_state, &mut fork.journaled_state);
    }

    *active_journaled_state = fork.journaled_state.clone();
}

/// Clones the account data from the `active_journaled_state`  into the `fork_journaled_state`
fn clone_journaled_state_data(
    addr: Address,
    active_journaled_state: &JournaledState,
    fork_journaled_state: &mut JournaledState,
) {
    if let Some(acc) = active_journaled_state.state.get(&addr).cloned() {
        trace!(?addr, "updating journaled_state account data");
        fork_journaled_state.state.insert(addr, acc);
    }
}

/// Clones the account data from the `active` db into the `ForkDB`
fn clone_db_account_data<ExtDB: DatabaseRef>(
    addr: Address,
    active: &CacheDB<ExtDB>,
    fork_db: &mut ForkDB,
) {
    trace!(?addr, "cloning database data");
    let acc = active.accounts.get(&addr).cloned().unwrap_or_default();
    if let Some(code) = active.contracts.get(&acc.info.code_hash).cloned() {
        fork_db.contracts.insert(acc.info.code_hash, code);
    }
    fork_db.accounts.insert(addr, acc);
}
