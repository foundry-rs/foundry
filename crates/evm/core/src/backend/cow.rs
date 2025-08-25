//! A wrapper around `Backend` that is clone-on-write used for fuzzing.

use super::BackendError;
use crate::{
    AsEnvMut, Env, EnvMut, InspectorExt,
    backend::{
        Backend, DatabaseExt, JournaledState, LocalForkId, RevertStateSnapshotAction,
        diagnostic::RevertDiagnostic,
    },
    fork::{CreateFork, ForkId},
};
use alloy_evm::Evm;
use alloy_genesis::GenesisAccount;
use alloy_primitives::{Address, B256, U256};
use alloy_rpc_types::TransactionRequest;
use eyre::WrapErr;
use foundry_fork_db::DatabaseError;
use revm::{
    Database, DatabaseCommit,
    bytecode::Bytecode,
    context_interface::result::ResultAndState,
    database::DatabaseRef,
    primitives::{HashMap as Map, hardfork::SpecId},
    state::{Account, AccountInfo},
};
use std::{borrow::Cow, collections::BTreeMap};

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
/// If they executed, it will require a clone of the initial input database.
/// This way we can support these cheatcodes cheaply without adding overhead for tests that
/// don't make use of them. Alternatively each test case would require its own `Backend` clone,
/// which would add significant overhead for large fuzz sets even if the Database is not big after
/// setup.
#[derive(Clone, Debug)]
pub struct CowBackend<'a> {
    /// The underlying `Backend`.
    ///
    /// No calls on the `CowBackend` will ever persistently modify the `backend`'s state.
    pub backend: Cow<'a, Backend>,
    /// Keeps track of whether the backed is already initialized
    is_initialized: bool,
    /// The [SpecId] of the current backend.
    spec_id: SpecId,
}

impl<'a> CowBackend<'a> {
    /// Creates a new `CowBackend` with the given `Backend`.
    pub fn new_borrowed(backend: &'a Backend) -> Self {
        Self { backend: Cow::Borrowed(backend), is_initialized: false, spec_id: SpecId::default() }
    }

    /// Executes the configured transaction of the `env` without committing state changes
    ///
    /// Note: in case there are any cheatcodes executed that modify the environment, this will
    /// update the given `env` with the new values.
    #[instrument(name = "inspect", level = "debug", skip_all)]
    pub fn inspect<I: InspectorExt>(
        &mut self,
        env: &mut Env,
        inspector: I,
    ) -> eyre::Result<ResultAndState> {
        // this is a new call to inspect with a new env, so even if we've cloned the backend
        // already, we reset the initialized state
        self.is_initialized = false;
        self.spec_id = env.evm_env.cfg_env.spec;

        let mut evm = crate::evm::new_evm_with_inspector(self, env.to_owned(), inspector);

        let res = evm.transact(env.tx.clone()).wrap_err("EVM error")?;

        *env = evm.as_env_mut().to_owned();

        Ok(res)
    }

    /// Returns whether there was a state snapshot failure in the backend.
    ///
    /// This is bubbled up from the underlying Copy-On-Write backend when a revert occurs.
    pub fn has_state_snapshot_failure(&self) -> bool {
        self.backend.has_state_snapshot_failure()
    }

    /// Returns a mutable instance of the Backend.
    ///
    /// If this is the first time this is called, the backed is cloned and initialized.
    fn backend_mut(&mut self, env: &EnvMut<'_>) -> &mut Backend {
        if !self.is_initialized {
            let backend = self.backend.to_mut();
            let mut env = env.to_owned();
            env.evm_env.cfg_env.spec = self.spec_id;
            backend.initialize(&env);
            self.is_initialized = true;
            return backend;
        }
        self.backend.to_mut()
    }

    /// Returns a mutable instance of the Backend if it is initialized.
    fn initialized_backend_mut(&mut self) -> Option<&mut Backend> {
        if self.is_initialized {
            return Some(self.backend.to_mut());
        }
        None
    }
}

impl DatabaseExt for CowBackend<'_> {
    fn snapshot_state(&mut self, journaled_state: &JournaledState, env: &mut EnvMut<'_>) -> U256 {
        self.backend_mut(env).snapshot_state(journaled_state, env)
    }

    fn revert_state(
        &mut self,
        id: U256,
        journaled_state: &JournaledState,
        current: &mut EnvMut<'_>,
        action: RevertStateSnapshotAction,
    ) -> Option<JournaledState> {
        self.backend_mut(current).revert_state(id, journaled_state, current, action)
    }

    fn delete_state_snapshot(&mut self, id: U256) -> bool {
        // delete state snapshot requires a previous snapshot to be initialized
        if let Some(backend) = self.initialized_backend_mut() {
            return backend.delete_state_snapshot(id);
        }
        false
    }

    fn delete_state_snapshots(&mut self) {
        if let Some(backend) = self.initialized_backend_mut() {
            backend.delete_state_snapshots()
        }
    }

    fn create_fork(&mut self, fork: CreateFork) -> eyre::Result<LocalForkId> {
        self.backend.to_mut().create_fork(fork)
    }

    fn create_fork_at_transaction(
        &mut self,
        fork: CreateFork,
        transaction: B256,
    ) -> eyre::Result<LocalForkId> {
        self.backend.to_mut().create_fork_at_transaction(fork, transaction)
    }

    fn select_fork(
        &mut self,
        id: LocalForkId,
        env: &mut EnvMut<'_>,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<()> {
        self.backend_mut(env).select_fork(id, env, journaled_state)
    }

    fn roll_fork(
        &mut self,
        id: Option<LocalForkId>,
        block_number: u64,
        env: &mut EnvMut<'_>,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<()> {
        self.backend_mut(env).roll_fork(id, block_number, env, journaled_state)
    }

    fn roll_fork_to_transaction(
        &mut self,
        id: Option<LocalForkId>,
        transaction: B256,
        env: &mut EnvMut<'_>,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<()> {
        self.backend_mut(env).roll_fork_to_transaction(id, transaction, env, journaled_state)
    }

    fn transact(
        &mut self,
        id: Option<LocalForkId>,
        transaction: B256,
        mut env: Env,
        journaled_state: &mut JournaledState,
        inspector: &mut dyn InspectorExt,
    ) -> eyre::Result<()> {
        self.backend_mut(&env.as_env_mut()).transact(
            id,
            transaction,
            env,
            journaled_state,
            inspector,
        )
    }

    fn transact_from_tx(
        &mut self,
        transaction: &TransactionRequest,
        mut env: Env,
        journaled_state: &mut JournaledState,
        inspector: &mut dyn InspectorExt,
    ) -> eyre::Result<()> {
        self.backend_mut(&env.as_env_mut()).transact_from_tx(
            transaction,
            env,
            journaled_state,
            inspector,
        )
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
        allocs: &BTreeMap<Address, GenesisAccount>,
        journaled_state: &mut JournaledState,
    ) -> Result<(), BackendError> {
        self.backend_mut(&Env::default().as_env_mut()).load_allocs(allocs, journaled_state)
    }

    fn clone_account(
        &mut self,
        source: &GenesisAccount,
        target: &Address,
        journaled_state: &mut JournaledState,
    ) -> Result<(), BackendError> {
        self.backend_mut(&Env::default().as_env_mut()).clone_account(
            source,
            target,
            journaled_state,
        )
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

    fn revoke_cheatcode_access(&mut self, account: &Address) -> bool {
        self.backend.to_mut().revoke_cheatcode_access(account)
    }

    fn has_cheatcode_access(&self, account: &Address) -> bool {
        self.backend.has_cheatcode_access(account)
    }

    fn set_blockhash(&mut self, block_number: U256, block_hash: B256) {
        self.backend.to_mut().set_blockhash(block_number, block_hash);
    }
}

impl DatabaseRef for CowBackend<'_> {
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

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        DatabaseRef::block_hash_ref(self.backend.as_ref(), number)
    }
}

impl Database for CowBackend<'_> {
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

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        DatabaseRef::block_hash_ref(self, number)
    }
}

impl DatabaseCommit for CowBackend<'_> {
    fn commit(&mut self, changes: Map<Address, Account>) {
        self.backend.to_mut().commit(changes)
    }
}
