use crate::{
    executor::{
        backend::{
            diagnostic::RevertDiagnostic, error::DatabaseError, Backend, DatabaseExt, LocalForkId,
        },
        fork::{CreateFork, ForkId},
        inspector::cheatcodes::Cheatcodes,
    },
    Address,
};
use ethers::prelude::{H256, U256};
use hashbrown::HashMap as Map;
use revm::{
    db::DatabaseRef, Account, AccountInfo, Bytecode, Database, Env, ExecutionResult, Inspector,
    JournaledState,
};
use std::borrow::Cow;
use tracing::trace;

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
}

// === impl FuzzBackendWrapper ===

impl<'a> FuzzBackendWrapper<'a> {
    pub fn new(backend: &'a Backend) -> Self {
        Self { backend: Cow::Borrowed(backend), is_initialized: false }
    }

    /// Executes the configured transaction of the `env` without committing state changes
    pub fn inspect_ref<INSP>(
        &mut self,
        env: &mut Env,
        mut inspector: INSP,
    ) -> (ExecutionResult, Map<Address, Account>)
    where
        INSP: Inspector<Self>,
    {
        // this is a new call to inspect with a new env, so even if we've cloned the backend
        // already, we reset the initialized state
        self.is_initialized = false;
        revm::evm_inner::<Self, true>(env, self, &mut inspector).transact()
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
        self.backend_mut(current).revert(id, journaled_state, current)
    }

    fn create_fork(&mut self, fork: CreateFork) -> eyre::Result<LocalForkId> {
        trace!("fuzz: create fork");
        self.backend.to_mut().create_fork(fork)
    }

    fn create_fork_at_transaction(
        &mut self,
        fork: CreateFork,
        transaction: H256,
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
        transaction: H256,
        env: &mut Env,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<()> {
        trace!(?id, ?transaction, "fuzz: roll fork to transaction");
        self.backend_mut(env).roll_fork_to_transaction(id, transaction, env, journaled_state)
    }

    fn transact(
        &mut self,
        id: Option<LocalForkId>,
        transaction: H256,
        env: &mut Env,
        journaled_state: &mut JournaledState,
        cheatcodes_inspector: Option<&mut Cheatcodes>,
    ) -> eyre::Result<()> {
        trace!(?id, ?transaction, "fuzz: execute transaction");
        self.backend_mut(env).transact(id, transaction, env, journaled_state, cheatcodes_inspector)
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

    fn basic(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        DatabaseRef::basic(self.backend.as_ref(), address)
    }

    fn code_by_hash(&self, code_hash: H256) -> Result<Bytecode, Self::Error> {
        DatabaseRef::code_by_hash(self.backend.as_ref(), code_hash)
    }

    fn storage(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        DatabaseRef::storage(self.backend.as_ref(), address, index)
    }

    fn block_hash(&self, number: U256) -> Result<H256, Self::Error> {
        DatabaseRef::block_hash(self.backend.as_ref(), number)
    }
}

impl<'a> Database for FuzzBackendWrapper<'a> {
    type Error = DatabaseError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        DatabaseRef::basic(self, address)
    }

    fn code_by_hash(&mut self, code_hash: H256) -> Result<Bytecode, Self::Error> {
        DatabaseRef::code_by_hash(self, code_hash)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        DatabaseRef::storage(self, address, index)
    }

    fn block_hash(&mut self, number: U256) -> Result<H256, Self::Error> {
        DatabaseRef::block_hash(self, number)
    }
}
