//! Foundry's execution lifecycle over evm2: fork switching, snapshots and nested execution.
//!
//! evm2's [`State`] owns transaction scratch, journaling, warmth, transient storage, logs and the
//! accepted overlay of the running context. This module owns everything Foundry layers on top:
//! which backing database is active, the accepted state of every inactive fork, user-visible
//! snapshots, and nested transactions. It replaces the corresponding parts of the REVM-era
//! `DatabaseExt`.
//!
//! # Execution phases
//!
//! evm2 allows replacing state and the block environment while a transaction runs, but not the
//! spec or runtime [`Version`](evm2::Version), which also carries the chain ID. Operations that
//! need a different spec or chain ID while a transaction is in flight apply everything else at
//! once and report [`EnvTransition::Deferred`]; [`ExecutionLifecycle::begin_transaction`] then
//! applies the deferred environment at the next transaction boundary. Callers must surface a
//! deferred transition rather than ignore it.
//!
//! Suspending a context accepts its in-flight writes, so a fork switch, like on the REVM backend,
//! is not undone when an enclosing call reverts.

use super::{
    BackingDb, EvmEnv, ForkInfo, ForkRegistry, FoundryEvmExt, FoundryEvmFactory, LifecycleSnapshot,
    Parked, evm_env,
};
use crate::{backend::RevertStateSnapshotAction, state_snapshot::StateSnapshots};
use alloy_consensus::transaction::Recovered;
use alloy_primitives::{KECCAK256_EMPTY, Log, U256};
use evm2::{
    Evm, EvmFeatures, EvmTypes, EvmTypesHost, ExecutedTx, TxResult,
    bytecode::Bytecode,
    evm::{Cache, DbResult, State},
};
use std::mem;

/// Whether a lifecycle operation runs between transactions or inside one, such as from a
/// cheatcode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionPhase {
    /// Between transactions: every part of the environment can change.
    Boundary,
    /// Inside a running transaction.
    InFlight,
}

/// How an environment change was applied.
#[must_use = "a deferred spec or chain ID change must be surfaced"]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvTransition {
    /// The whole environment is in effect.
    Applied,
    /// State and block are in effect; the spec or chain ID takes effect at the next transaction
    /// boundary.
    Deferred,
}

/// Fork, snapshot and nested-execution lifecycle for one EVM network family.
///
/// `C` is caller-owned state captured with every snapshot, such as cheatcode state.
pub struct ExecutionLifecycle<F: FoundryEvmFactory, C = ()> {
    factory: F,
    forks: ForkRegistry<F::Types>,
    snapshots: StateSnapshots<LifecycleSnapshot<F::Types, C>>,
    pending_env: Option<EvmEnv<F::Types>>,
}

impl<F: FoundryEvmFactory, C> std::fmt::Debug for ExecutionLifecycle<F, C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionLifecycle").field("forks", &self.forks).finish_non_exhaustive()
    }
}

impl<F: FoundryEvmFactory, C: Clone> ExecutionLifecycle<F, C> {
    /// Creates a lifecycle whose active context is local execution over `db`.
    pub fn new(factory: F, db: impl BackingDb, env: EvmEnv<F::Types>) -> Self {
        let ext = factory.configure_for(&db, &env).evm_ext(&env);
        Self {
            factory,
            forks: ForkRegistry::new(Parked::new(Box::new(db), env, ext)),
            snapshots: StateSnapshots::default(),
            pending_env: None,
        }
    }

    /// Returns the factory.
    pub const fn factory(&self) -> &F {
        &self.factory
    }

    /// Returns the fork registry.
    pub const fn forks(&self) -> &ForkRegistry<F::Types> {
        &self.forks
    }

    /// Returns the fork registry mutably, for persistent-account changes.
    pub const fn forks_mut(&mut self) -> &mut ForkRegistry<F::Types> {
        &mut self.forks
    }

    /// Creates an EVM for the active context as it was when last suspended.
    ///
    /// Call this once; afterwards the running EVM holds the active context's state.
    pub fn create_evm(&mut self) -> Evm<'static, F::Types> {
        let active = self.forks.active();
        let context = self.forks.context_mut(active).expect("active context exists");
        let db = context.db.clone_backing();
        let env = context.env;
        let cache = context.cache.take();
        let ext = context.ext.clone();
        let factory = self.factory.configure_for(&db, &env);
        match cache {
            None => factory.create_evm(db, env),
            Some(cache) => {
                let mut evm = factory.new_evm(db, env);
                evm.state_mut().overlay_db_mut().cache = cache;
                *evm.ext_mut() = ext;
                evm
            }
        }
    }

    /// Registers a fork without selecting it and returns its local fork ID.
    pub fn create_fork(
        &mut self,
        info: ForkInfo,
        db: impl BackingDb,
        env: EvmEnv<F::Types>,
    ) -> U256 {
        let ext = self.factory.configure_for(&db, &env).evm_ext(&env);
        self.forks.insert(info, Parked::new(Box::new(db), env, ext))
    }

    /// Makes `id` the running context, or the local context for `None`.
    ///
    /// Persistent accounts carry their accepted state into the target.
    pub fn select_fork(
        &mut self,
        evm: &mut Evm<'_, F::Types>,
        id: Option<U256>,
        phase: ExecutionPhase,
    ) -> eyre::Result<EnvTransition> {
        if self.forks.context(id).is_none() {
            eyre::bail!("fork {} does not exist", id.unwrap_or_default());
        }
        if id == self.forks.active() {
            return Ok(EnvTransition::Applied);
        }
        let (source, logs) = self.suspend(evm);
        self.forks.active = id;
        Ok(self.resume(evm, logs, &source, phase))
    }

    /// Moves fork `id`, or the running context for `None`, onto a new backing database and
    /// environment, such as a later block.
    ///
    /// The fork's accepted state is discarded except for persistent accounts.
    pub fn roll_fork(
        &mut self,
        evm: &mut Evm<'_, F::Types>,
        id: Option<U256>,
        db: impl BackingDb,
        env: EvmEnv<F::Types>,
        phase: ExecutionPhase,
    ) -> eyre::Result<EnvTransition> {
        let active = self.forks.active();
        let id = id.or(active);
        let ext = self.factory.configure_for(&db, &env).evm_ext(&env);
        let Some(context) = self.forks.context_mut(id) else {
            eyre::bail!("fork {} does not exist", id.unwrap_or_default());
        };
        *context = Parked::new(Box::new(db), env, ext);
        if id != active {
            return Ok(EnvTransition::Applied);
        }
        let (source, logs) = self.suspend(evm);
        // Suspending saved the running state into the context being replaced; drop it.
        self.forks.context_mut(id).expect("active context exists").cache = None;
        Ok(self.resume(evm, logs, &source, phase))
    }

    /// Applies an environment change deferred by an in-flight operation. Call before every
    /// transaction.
    pub fn begin_transaction(&mut self, evm: &mut Evm<'_, F::Types>) {
        if let Some(mut env) = self.pending_env.take() {
            env.block = *evm.block();
            let db = self.forks.active_db();
            self.factory.configure_for(&db, &env).set_env(evm, env);
        }
    }

    /// Captures the running context, every fork, and `extra`, and returns the snapshot ID.
    pub fn snapshot(&mut self, evm: &Evm<'_, F::Types>, extra: C) -> U256 {
        self.snapshots.insert(LifecycleSnapshot {
            state: evm.state().snapshot(),
            forks: self.forks.clone(),
            env: evm_env(evm),
            ext: evm.ext().clone(),
            extra,
        })
    }

    /// Restores snapshot `id` and returns its `extra`, or `None` if it does not exist.
    ///
    /// Logs emitted since the snapshot are kept, like on the REVM backend.
    pub fn revert(
        &mut self,
        evm: &mut Evm<'_, F::Types>,
        id: U256,
        action: RevertStateSnapshotAction,
        phase: ExecutionPhase,
    ) -> Option<(C, EnvTransition)> {
        let snapshot = if action.is_keep() {
            self.snapshots.get(id).cloned()
        } else {
            self.snapshots.remove(id)
        }?;
        let LifecycleSnapshot { state, forks, env, ext, extra } = snapshot;
        let next_id = self.forks.next_id;
        self.forks = forks;
        self.forks.next_id = next_id;
        let db = self.forks.active_db();
        let logs = mem::take(evm.state_mut().logs_mut());
        let mut state = state.into_state(db.clone_backing());
        *state.logs_mut() = logs;
        *evm.state_mut() = state;
        *evm.ext_mut() = ext;
        let factory = self.factory.configure_for(&db, &env);
        let transition = self.apply_env(&factory, evm, env, phase);
        Some((extra, transition))
    }

    /// Deletes a snapshot, returning whether it existed.
    pub fn delete_snapshot(&mut self, id: U256) -> bool {
        self.snapshots.remove_at(id).is_some()
    }

    /// Deletes every snapshot.
    pub fn delete_snapshots(&mut self) {
        self.snapshots.clear();
    }

    /// Executes `tx` as a separate transaction over the running transaction's current state, such
    /// as for `vm.transact`, and merges its state changes into the running transaction on success.
    ///
    /// The merged writes are journaled, so they revert with the enclosing call. A failed
    /// transaction changes nothing.
    pub fn nested_transaction(
        &self,
        evm: &mut Evm<'_, F::Types>,
        tx: &Recovered<<F::Types as EvmTypesHost>::Tx>,
    ) -> eyre::Result<TxResult<F::Types>> {
        let db = self.forks.active_db();
        let mut state = evm.state().snapshot().into_state(db.clone_backing());
        state.commit_transaction();
        state.clear_transaction_state();
        let env = evm_env(evm);
        let mut nested = self.factory.configure_for(&db, &env).new_evm(db, env);
        *nested.state_mut() = state;
        *nested.ext_mut() = evm.ext().clone();
        nested.ext_mut().for_transaction(tx);
        let executed = nested.transact(tx).map_err(|err| eyre::eyre!("{err:?}"))?;
        let result = executed.result().clone();
        if result.status {
            merge_changes(evm.state_mut(), &executed)?;
        }
        Ok(result)
    }

    /// Accepts the running state and stores it, with the running environment, in the active
    /// context. Returns the accepted state and the in-flight transaction's logs.
    fn suspend(&mut self, evm: &mut Evm<'_, F::Types>) -> (Cache, Vec<Log>) {
        let state = evm.state_mut();
        state.commit_transaction();
        let logs = mem::take(state.logs_mut());
        let cache = state.overlay_db().cache.clone();
        let context = self.forks.context_mut(self.forks.active()).expect("active context exists");
        context.cache = Some(cache.clone());
        context.env = evm_env(evm);
        context.ext = evm.ext().clone();
        (cache, logs)
    }

    /// Installs the active context into `evm`, carrying persistent accounts from `source`.
    fn resume(
        &mut self,
        evm: &mut Evm<'_, F::Types>,
        logs: Vec<Log>,
        source: &Cache,
        phase: ExecutionPhase,
    ) -> EnvTransition {
        let active = self.forks.active();
        let context = self.forks.context_mut(active).expect("active context exists");
        let fresh = context.cache.is_none();
        // The running EVM owns the active context's accepted state.
        let mut cache = context.cache.take().unwrap_or_default();
        let db = context.db.clone_backing();
        let env = context.env;
        let ext = context.ext.clone();
        self.forks.merge_persistent(source, &mut cache);

        let mut state = State::new(db.clone_backing());
        state.overlay_db_mut().cache = cache;
        *state.logs_mut() = logs;
        *evm.state_mut() = state;
        *evm.ext_mut() = ext;

        let factory = self.factory.configure_for(&db, &env);
        let transition = self.apply_env(&factory, evm, env, phase);
        evm.warm_precompiles();
        if evm.feature(EvmFeatures::EIP3651) {
            let beneficiary = evm.block().beneficiary;
            evm.state_mut().prewarm(&beneficiary);
        }
        if fresh {
            factory.initialize_evm(evm, db.is_forked_mode());
        }
        transition
    }

    /// Applies `env`, deferring a spec or chain ID change while a transaction is in flight.
    fn apply_env(
        &mut self,
        factory: &F,
        evm: &mut Evm<'_, F::Types>,
        env: EvmEnv<F::Types>,
        phase: ExecutionPhase,
    ) -> EnvTransition {
        let current = evm_env(evm);
        if current.spec == env.spec && current.chain_id() == env.chain_id() {
            evm.set_block(env.block);
            self.pending_env = None;
            return EnvTransition::Applied;
        }
        match phase {
            ExecutionPhase::Boundary => {
                factory.set_env(evm, env);
                self.pending_env = None;
                EnvTransition::Applied
            }
            ExecutionPhase::InFlight => {
                evm.set_block(env.block);
                self.pending_env = Some(env);
                EnvTransition::Deferred
            }
        }
    }
}

/// Runs `f` as a nested frame of the running transaction: its writes are rolled back if it fails.
pub fn nested_frame<T: EvmTypes, R, E>(
    evm: &mut Evm<'_, T>,
    f: impl FnOnce(&mut Evm<'_, T>) -> Result<R, E>,
) -> Result<R, E> {
    let checkpoint = evm.state().checkpoint();
    let result = f(evm);
    if result.is_err() {
        let features = evm.version().features;
        evm.state_mut().rollback(checkpoint, features);
    }
    result
}

/// Applies a nested transaction's state changes to `state` as journaled writes.
fn merge_changes<T: EvmTypesHost>(
    state: &mut State<'_>,
    executed: &ExecutedTx<'_, '_, T>,
) -> DbResult<()> {
    for change in executed.account_changes().filter(|change| change.is_changed()) {
        let address = change.address;
        {
            let mut account = state.account(&address)?;
            match change.current {
                Some(info) => {
                    account.set_balance(info.balance);
                    account.set_nonce(info.nonce);
                    if let Some((code_hash, code)) = change.code {
                        account.set_code(code_hash, code.clone());
                    }
                }
                None => {
                    account.set_balance(U256::ZERO);
                    account.set_nonce(0);
                    account.set_code(KECCAK256_EMPTY, Bytecode::default());
                }
            }
        }
        if change.current.is_none() || change.storage.wiped {
            state.storage(&address)?.wipe();
        }
        for (key, slot) in change.storage.changed_slots() {
            state.storage_slot(&address, *key)?.set(slot.current);
        }
    }
    Ok(())
}
