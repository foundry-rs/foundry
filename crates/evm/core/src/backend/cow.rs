//! A wrapper around `Backend` that is clone-on-write used for fuzzing.

use super::BackendError;
use crate::{
    FoundryInspectorExt,
    backend::{
        Backend, DatabaseExt, JournaledState, LocalForkId, RevertStateSnapshotAction,
        diagnostic::RevertDiagnostic,
    },
    evm::{
        EvmEnvFor, FoundryContextFor, FoundryEvmFactory, FoundryEvmNetwork, HaltReasonFor, SpecFor,
        TxEnvFor,
    },
    fork::{CreateFork, ForkId},
};
use alloy_evm::Evm;
use alloy_genesis::GenesisAccount;
use alloy_primitives::{Address, B256, TxKind, U256};
use eyre::WrapErr;
use foundry_fork_db::DatabaseError;
use revm::{
    Database, DatabaseCommit,
    bytecode::Bytecode,
    context::{ContextTr, Transaction},
    context_interface::result::ResultAndState,
    database::DatabaseRef,
    primitives::AddressMap,
    state::{Account, AccountInfo, EvmState},
};
use std::{borrow::Cow, collections::BTreeMap, fmt::Debug};

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
pub struct CowBackend<'a, FEN: FoundryEvmNetwork> {
    /// The underlying `Backend`.
    ///
    /// No calls on the `CowBackend` will ever persistently modify the `backend`'s state.
    pub backend: Cow<'a, Backend<FEN>>,
    /// Pending initialization params for the backend on first mutable access.
    /// `None` means the backend has already been initialized for the current call.
    pending_init: Option<(SpecFor<FEN>, Address, TxKind)>,
}

impl<FEN: FoundryEvmNetwork> Clone for CowBackend<'_, FEN> {
    fn clone(&self) -> Self {
        Self { backend: self.backend.clone(), pending_init: self.pending_init }
    }
}

impl<FEN: FoundryEvmNetwork> Debug for CowBackend<'_, FEN> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CowBackend")
            .field("backend", &self.backend)
            .field("pending_init", &self.pending_init)
            .finish()
    }
}

impl<'a, FEN: FoundryEvmNetwork> CowBackend<'a, FEN> {
    /// Creates a new `CowBackend` with the given `Backend`.
    pub const fn new_borrowed(backend: &'a Backend<FEN>) -> Self {
        Self { backend: Cow::Borrowed(backend), pending_init: None }
    }

    /// Executes the configured transaction of the `env` without committing state changes
    ///
    /// Note: in case there are any cheatcodes executed that modify the environment, this will
    /// update the given `env` with the new values.
    #[instrument(name = "inspect", level = "debug", skip_all)]
    pub fn inspect<I: for<'db> FoundryInspectorExt<FoundryContextFor<'db, FEN>>>(
        &mut self,
        evm_env: &mut EvmEnvFor<FEN>,
        tx_env: &mut TxEnvFor<FEN>,
        inspector: I,
    ) -> eyre::Result<ResultAndState<HaltReasonFor<FEN>>> {
        // this is a new call to inspect with a new env, so even if we've cloned the backend
        // already, we reset the initialized state
        self.pending_init = Some((evm_env.cfg_env.spec, tx_env.caller(), tx_env.kind()));

        let mut evm = FEN::EvmFactory::default().create_foundry_evm_with_inspector(
            self,
            evm_env.clone(),
            inspector,
        );

        let res = evm.transact(tx_env.clone()).wrap_err("EVM error")?;

        *tx_env = evm.tx().clone();
        *evm_env = evm.finish().1;

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
    fn backend_mut(&mut self) -> &mut Backend<FEN> {
        if let Some((spec_id, caller, tx_kind)) = self.pending_init.take() {
            let backend = self.backend.to_mut();
            backend.initialize(spec_id, caller, tx_kind);
            return backend;
        }
        self.backend.to_mut()
    }

    /// Returns a mutable instance of the Backend if it is initialized.
    fn initialized_backend_mut(&mut self) -> Option<&mut Backend<FEN>> {
        if self.pending_init.is_none() {
            return Some(self.backend.to_mut());
        }
        None
    }
}

impl<FEN: FoundryEvmNetwork> DatabaseExt<FEN::EvmFactory> for CowBackend<'_, FEN> {
    fn snapshot_state(
        &mut self,
        journaled_state: &JournaledState,
        evm_env: &EvmEnvFor<FEN>,
    ) -> U256 {
        self.backend_mut().snapshot_state(journaled_state, evm_env)
    }

    fn revert_state(
        &mut self,
        id: U256,
        journaled_state: &JournaledState,
        evm_env: &mut EvmEnvFor<FEN>,
        caller: Address,
        action: RevertStateSnapshotAction,
    ) -> Option<JournaledState> {
        self.backend_mut().revert_state(id, journaled_state, evm_env, caller, action)
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
        evm_env: &mut EvmEnvFor<FEN>,
        tx_env: &mut TxEnvFor<FEN>,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<()> {
        self.backend_mut().select_fork(id, evm_env, tx_env, journaled_state)
    }

    fn roll_fork(
        &mut self,
        id: Option<LocalForkId>,
        block_number: u64,
        evm_env: &mut EvmEnvFor<FEN>,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<()> {
        self.backend_mut().roll_fork(id, block_number, evm_env, journaled_state)
    }

    fn roll_fork_to_transaction(
        &mut self,
        id: Option<LocalForkId>,
        transaction: B256,
        evm_env: &mut EvmEnvFor<FEN>,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<()> {
        self.backend_mut().roll_fork_to_transaction(id, transaction, evm_env, journaled_state)
    }

    fn transact(
        &mut self,
        id: Option<LocalForkId>,
        transaction: B256,
        evm_env: EvmEnvFor<FEN>,
        journaled_state: &mut JournaledState,
        inspector: &mut dyn for<'db> FoundryInspectorExt<
            <FEN::EvmFactory as FoundryEvmFactory>::FoundryContext<'db>,
        >,
    ) -> eyre::Result<()> {
        self.backend_mut().transact(id, transaction, evm_env, journaled_state, inspector)
    }

    fn transact_from_tx(
        &mut self,
        tx_env: TxEnvFor<FEN>,
        evm_env: EvmEnvFor<FEN>,
        journaled_state: &mut JournaledState,
        inspector: &mut dyn for<'db> FoundryInspectorExt<
            <FEN::EvmFactory as FoundryEvmFactory>::FoundryContext<'db>,
        >,
    ) -> eyre::Result<()> {
        self.backend_mut().transact_from_tx(tx_env, evm_env, journaled_state, inspector)
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

    fn diagnose_revert(&self, callee: Address, evm_state: &EvmState) -> Option<RevertDiagnostic> {
        self.backend.diagnose_revert(callee, evm_state)
    }

    fn load_allocs(
        &mut self,
        allocs: &BTreeMap<Address, GenesisAccount>,
        journaled_state: &mut JournaledState,
    ) -> Result<(), BackendError> {
        self.backend.to_mut().load_allocs(allocs, journaled_state)
    }

    fn clone_account(
        &mut self,
        source: &GenesisAccount,
        target: &Address,
        journaled_state: &mut JournaledState,
    ) -> Result<(), BackendError> {
        self.backend.to_mut().clone_account(source, target, journaled_state)
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

impl<FEN: FoundryEvmNetwork> DatabaseRef for CowBackend<'_, FEN> {
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

impl<FEN: FoundryEvmNetwork> Database for CowBackend<'_, FEN> {
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

impl<FEN: FoundryEvmNetwork> DatabaseCommit for CowBackend<'_, FEN> {
    fn commit(&mut self, changes: AddressMap<Account>) {
        self.backend.to_mut().commit(changes)
    }
}
