use crate::{
    executor::{
        backend::{diagnostic::RevertDiagnostic, Backend, DatabaseExt, LocalForkId},
        fork::{CreateFork, ForkId},
    },
    Address,
};
use bytes::Bytes;
use ethers::prelude::{H160, H256, U256};
use hashbrown::HashMap as Map;
use revm::{
    db::DatabaseRef, Account, AccountInfo, Database, Env, Inspector, Log, Return, SubRoutine,
    TransactOut,
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
/// Entire purpose of this type is for fuzzing. A test function fuzzer will repeatedly execute  the
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
}

// === impl FuzzBackendWrapper ===

impl<'a> FuzzBackendWrapper<'a> {
    pub fn new(backend: &'a Backend) -> Self {
        Self { backend: Cow::Borrowed(backend) }
    }

    /// Executes the configured transaction of the `env` without committing state changes
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

impl<'a> DatabaseExt for FuzzBackendWrapper<'a> {
    fn snapshot(&mut self, subroutine: &SubRoutine, env: &Env) -> U256 {
        trace!("fuzz: create snapshot");
        self.backend.to_mut().snapshot(subroutine, env)
    }

    fn revert(
        &mut self,
        id: U256,
        subroutine: &SubRoutine,
        current: &mut Env,
    ) -> Option<SubRoutine> {
        trace!(?id, "fuzz: revert snapshot");
        self.backend.to_mut().revert(id, subroutine, current)
    }

    fn create_fork(
        &mut self,
        fork: CreateFork,
        subroutine: &SubRoutine,
    ) -> eyre::Result<LocalForkId> {
        trace!("fuzz: create fork");
        self.backend.to_mut().create_fork(fork, subroutine)
    }

    fn select_fork(
        &mut self,
        id: LocalForkId,
        env: &mut Env,
        subroutine: &mut SubRoutine,
    ) -> eyre::Result<()> {
        trace!(?id, "fuzz: select fork");
        self.backend.to_mut().select_fork(id, env, subroutine)
    }

    fn roll_fork(
        &mut self,
        env: &mut Env,
        block_number: U256,
        id: Option<LocalForkId>,
    ) -> eyre::Result<()> {
        trace!(?id, ?block_number, "fuzz: roll fork");
        self.backend.to_mut().roll_fork(env, block_number, id)
    }

    fn active_fork_id(&self) -> Option<LocalForkId> {
        self.backend.active_fork_id()
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
        subroutine: &SubRoutine,
    ) -> Option<RevertDiagnostic> {
        self.backend.diagnose_revert(callee, subroutine)
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
}

impl<'a> DatabaseRef for FuzzBackendWrapper<'a> {
    fn basic(&self, address: H160) -> AccountInfo {
        DatabaseRef::basic(self.backend.as_ref(), address)
    }

    fn code_by_hash(&self, code_hash: H256) -> Bytes {
        DatabaseRef::code_by_hash(self.backend.as_ref(), code_hash)
    }

    fn storage(&self, address: H160, index: U256) -> U256 {
        DatabaseRef::storage(self.backend.as_ref(), address, index)
    }

    fn block_hash(&self, number: U256) -> H256 {
        DatabaseRef::block_hash(self.backend.as_ref(), number)
    }
}

impl<'a> Database for FuzzBackendWrapper<'a> {
    fn basic(&mut self, address: H160) -> AccountInfo {
        DatabaseRef::basic(self, address)
    }
    fn code_by_hash(&mut self, code_hash: H256) -> Bytes {
        DatabaseRef::code_by_hash(self, code_hash)
    }
    fn storage(&mut self, address: H160, index: U256) -> U256 {
        DatabaseRef::storage(self, address, index)
    }

    fn block_hash(&mut self, number: U256) -> H256 {
        DatabaseRef::block_hash(self, number)
    }
}
