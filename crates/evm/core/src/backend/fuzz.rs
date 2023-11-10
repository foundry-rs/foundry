//! A wrapper around `Backend` that is clone-on-write used for fuzzing.

use crate::{
    backend::{
        diagnostic::RevertDiagnostic, error::DatabaseError, Backend, DatabaseExt, LocalForkId,
    },
    fork::{CreateFork, ForkId},
};
use alloy_primitives::{Address, B256, U256};
use ethers_core::utils::GenesisAccount;
use revm::{
    db::DatabaseRef,
    primitives::{AccountInfo, Bytecode, Env, ResultAndState},
    Database, Inspector, JournaledState,
};
use std::{borrow::Cow, collections::HashMap};

/// A wrapper around `Backend` that ensures only `revm::DatabaseRef` functions are called.
///
/// Any changes made during its existence that affect the caching layer of the underlying Database
/// will result in a clone of the initial Database. Therefore, this backend type is basically
/// a clone-on-write `Backend`, where cloning is only necessary if cheatcodes will modify the
/// `Backend`
///
/// Entire purpose of this type is for fuzzing. A test function fuzzer will repeatedly execute the
/// function via immutable raw (no state changes) calls.
///
/// **N.B.**: we're assuming cheatcodes that alter the state (like multi fork swapping) are niche.
/// If they executed during fuzzing, it will require a clone of the initial input database. This way
/// we can support these cheatcodes in fuzzing cheaply without adding overhead for fuzz tests that
/// don't make use of them. Alternatively each test case would require its own `Backend` clone,
/// which would add significant overhead for large fuzz sets even if the Database is not big after
/// setup.
#[derive(Debug, Clone)]
pub struct FuzzBackendWrapper<'a> {
    /// The underlying immutable `Backend`
    ///
    /// No calls on the `FuzzBackendWrapper` will ever persistently modify the `backend`'s state.
    pub backend: Cow<'a, Backend>,
    /// Keeps track of whether the backed is already intialized
    is_initialized: bool,
    /// Keeps track of wheter there was a snapshot failure.
    ///
    /// Necessary as the backend is dropped after usage, but we'll need to persist
    /// the snapshot failure anyhow.
    has_snapshot_failure: bool,
}

impl<'a> FuzzBackendWrapper<'a> {
    pub fn new(backend: &'a Backend) -> Self {
        Self { backend: Cow::Borrowed(backend), is_initialized: false, has_snapshot_failure: false }
    }

    /// Executes the configured transaction of the `env` without committing state changes
    pub fn inspect_ref<INSP>(
        &mut self,
        env: &mut Env,
        mut inspector: INSP,
    ) -> eyre::Result<ResultAndState>
    where
        INSP: Inspector<Self>,
    {
        // this is a new call to inspect with a new env, so even if we've cloned the backend
        // already, we reset the initialized state
        self.is_initialized = false;
        match revm::evm_inner::<Self>(env, self, Some(&mut inspector)).transact() {
            Ok(result) => Ok(result),
            Err(e) => eyre::bail!("fuzz: failed to inspect: {:?}", e),
        }
    }

    /// Returns whether there was a snapshot failure in the fuzz backend.
    ///
    /// This is bubbled up from the underlying Copy-On-Write backend when a revert occurs.
    pub fn has_snapshot_failure(&self) -> bool {
        self.has_snapshot_failure
    }

    /// Sets whether there was a snapshot failure in the fuzz backend.
    ///
    /// This is bubbled up from the underlying Copy-On-Write backend when a revert occurs.
    pub fn set_snapshot_failure(&mut self, has_snapshot_failure: bool) {
        self.has_snapshot_failure = has_snapshot_failure;
    }

    /// Returns a mutable instance of the Backend.
    ///
    /// If this is the first time this is called, the backed is cloned and initialized.
    fn backend_mut(&mut self, env: &Env) -> &mut Backend {
        if !self.is_initialized {
            let backend = self.backend.to_mut();
            backend.initialize(env);
            self.is_initialized = true;
            return backend
        }
        self.backend.to_mut()
    }
}

impl<'a> DatabaseExt for FuzzBackendWrapper<'a> {
    fn snapshot(&mut self, journaled_state: &JournaledState, env: &Env) -> U256 {
        trace!("fuzz: create snapshot");
        self.backend_mut(env).snapshot(journaled_state, env)
    }

    fn revert(
        &mut self,
        id: U256,
        journaled_state: &JournaledState,
        current: &mut Env,
    ) -> Option<JournaledState> {
        trace!(?id, "fuzz: revert snapshot");
        let journaled_state = self.backend_mut(current).revert(id, journaled_state, current);
        // Persist the snapshot failure in the fuzz backend, as the underlying backend state is lost
        // after the call.
        self.set_snapshot_failure(self.has_snapshot_failure || self.backend.has_snapshot_failure());
        journaled_state
    }

    fn create_fork(&mut self, fork: CreateFork) -> eyre::Result<LocalForkId> {
        trace!("fuzz: create fork");
        self.backend.to_mut().create_fork(fork)
    }

    fn create_fork_at_transaction(
        &mut self,
        fork: CreateFork,
        transaction: B256,
    ) -> eyre::Result<LocalForkId> {
        trace!(?transaction, "fuzz: create fork at");
        self.backend.to_mut().create_fork_at_transaction(fork, transaction)
    }

    fn select_fork(
        &mut self,
        id: LocalForkId,
        env: &mut Env,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<()> {
        trace!(?id, "fuzz: select fork");
        self.backend_mut(env).select_fork(id, env, journaled_state)
    }

    fn roll_fork(
        &mut self,
        id: Option<LocalForkId>,
        block_number: U256,
        env: &mut Env,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<()> {
        trace!(?id, ?block_number, "fuzz: roll fork");
        self.backend_mut(env).roll_fork(id, block_number, env, journaled_state)
    }

    fn roll_fork_to_transaction(
        &mut self,
        id: Option<LocalForkId>,
        transaction: B256,
        env: &mut Env,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<()> {
        trace!(?id, ?transaction, "fuzz: roll fork to transaction");
        self.backend_mut(env).roll_fork_to_transaction(id, transaction, env, journaled_state)
    }

    fn transact<I: Inspector<Backend>>(
        &mut self,
        id: Option<LocalForkId>,
        transaction: B256,
        env: &mut Env,
        journaled_state: &mut JournaledState,
        inspector: &mut I,
    ) -> eyre::Result<()> {
        trace!(?id, ?transaction, "fuzz: execute transaction");
        self.backend_mut(env).transact(id, transaction, env, journaled_state, inspector)
    }

    fn active_fork_id(&self) -> Option<LocalForkId> {
        self.backend.active_fork_id()
    }

    fn active_fork_url(&self) -> Option<String> {
        self.backend.active_fork_url()
    }

    fn ensure_fork(&self, id: Option<LocalForkId>) -> eyre::Result<LocalForkId> {
        self.backend.ensure_fork(id)
    }

    fn ensure_fork_id(&self, id: LocalForkId) -> eyre::Result<&ForkId> {
        self.backend.ensure_fork_id(id)
    }

    fn diagnose_revert(
        &self,
        callee: Address,
        journaled_state: &JournaledState,
    ) -> Option<RevertDiagnostic> {
        self.backend.diagnose_revert(callee, journaled_state)
    }

    fn load_allocs(
        &mut self,
        allocs: &HashMap<Address, GenesisAccount>,
        journaled_state: &mut JournaledState,
    ) -> Result<(), DatabaseError> {
        self.backend_mut(&Env::default()).load_allocs(allocs, journaled_state)
    }

    fn is_persistent(&self, acc: &Address) -> bool {
        self.backend.is_persistent(acc)
    }

    fn remove_persistent_account(&mut self, account: &Address) -> bool {
        self.backend.to_mut().remove_persistent_account(account)
    }

    fn add_persistent_account(&mut self, account: Address) -> bool {
        self.backend.to_mut().add_persistent_account(account)
    }

    fn allow_cheatcode_access(&mut self, account: Address) -> bool {
        self.backend.to_mut().allow_cheatcode_access(account)
    }

    fn revoke_cheatcode_access(&mut self, account: Address) -> bool {
        self.backend.to_mut().revoke_cheatcode_access(account)
    }

    fn has_cheatcode_access(&self, account: Address) -> bool {
        self.backend.has_cheatcode_access(account)
    }
}

impl<'a> DatabaseRef for FuzzBackendWrapper<'a> {
    type Error = DatabaseError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        DatabaseRef::basic_ref(self.backend.as_ref(), address)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        DatabaseRef::code_by_hash_ref(self.backend.as_ref(), code_hash)
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        DatabaseRef::storage_ref(self.backend.as_ref(), address, index)
    }

    fn block_hash_ref(&self, number: U256) -> Result<B256, Self::Error> {
        DatabaseRef::block_hash_ref(self.backend.as_ref(), number)
    }
}

impl<'a> Database for FuzzBackendWrapper<'a> {
    type Error = DatabaseError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        DatabaseRef::basic_ref(self, address)
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        DatabaseRef::code_by_hash_ref(self, code_hash)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        DatabaseRef::storage_ref(self, address, index)
    }

    fn block_hash(&mut self, number: U256) -> Result<B256, Self::Error> {
        DatabaseRef::block_hash_ref(self, number)
    }
}
