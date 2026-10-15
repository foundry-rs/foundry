//! Foundry's execution lifecycle over evm2: fork switching, snapshots and nested execution.
//!
//! evm2's [`State`] owns transaction scratch, journaling, warmth, transient storage, logs and the
//! accepted overlay of the running context. This module owns everything Foundry layers on top:
//! which backing database is active, the accepted state of every inactive fork, user-visible
//! snapshots, and nested transactions.
//!
//! # Executions
//!
//! [`ExecutionLifecycle::create_evm`] builds an EVM over the active context without copying its
//! accepted state: the EVM reads it through a shared view and its overlay only collects what the
//! execution reads and writes. [`ExecutionLifecycle::park`] accepts the EVM's state back into the
//! active context, after a commit; dropping the EVM instead discards it, as for calls. Lifecycle
//! operations that capture or replace the running state (fork switches, snapshots, nested
//! transactions) first materialize the view into the EVM's overlay, once.
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
//! Suspending a context accepts its in-flight writes, so a fork switch is not undone when an
//! enclosing call reverts.

use super::{
    AcceptedView, BackingDb, EvmEnv, ForkInfo, ForkRegistry, FoundryEvmExt, FoundryEvmFactory,
    LifecycleSnapshot, Parked, evm_env,
};
use crate::{backend::RevertStateSnapshotAction, state_snapshot::StateSnapshots};
use alloy_consensus::transaction::Recovered;
use alloy_primitives::{Address, KECCAK256_EMPTY, Log, U256};
use evm2::{
    Evm, EvmFeatures, EvmTypes, EvmTypesHost, ExecutedTx, ExecutionError, Inspector, TxResult,
    bytecode::Bytecode,
    env::TxEnv,
    evm::{Cache, DbResult, EmptyDB, PrewarmSet, State},
    interpreter::{Host, Message, MessageResult},
};
use std::{mem, sync::Arc};

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

/// Environment, inspector and network state of a nested transaction.
pub struct NestedTx<'a, T: EvmTypesHost> {
    /// Environment to execute with; the running environment if `None`.
    pub env: Option<EvmEnv<T>>,
    /// Network extension state to execute with; derived from the running EVM if `None`.
    pub ext: Option<T::EvmExt>,
    /// Inspector observing the nested transaction.
    pub inspector: Option<&'a mut dyn Inspector<T>>,
}

impl<T: EvmTypesHost> Default for NestedTx<'_, T> {
    fn default() -> Self {
        Self { env: None, ext: None, inspector: None }
    }
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

impl<F: FoundryEvmFactory, C: Clone> Clone for ExecutionLifecycle<F, C> {
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
            forks: self.forks.clone(),
            snapshots: self.snapshots.clone(),
            pending_env: self.pending_env,
        }
    }
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

    /// Returns the active context.
    pub fn active(&self) -> &Parked<F::Types> {
        self.forks.context(self.forks.active()).expect("active context exists")
    }

    /// Returns the active context mutably.
    ///
    /// Use it between executions only: a running EVM holds its own view of the context.
    pub fn active_mut(&mut self) -> &mut Parked<F::Types> {
        let active = self.forks.active();
        self.forks.context_mut(active).expect("active context exists")
    }

    /// Returns the active context's accepted state for modification between executions,
    /// initializing the context first if it never ran.
    pub fn accepted_mut(&mut self) -> &mut Cache {
        if self.active().cache.is_none() {
            let evm = self.create_evm();
            self.park(evm);
        }
        Arc::make_mut(self.active_mut().cache.as_mut().expect("context is initialized"))
    }

    /// Returns a database reading the active context's accepted state over its backing database,
    /// for reads between executions.
    pub fn accepted_db(&self) -> impl BackingDb + use<F, C> {
        let context = self.active();
        AcceptedView {
            cache: context.cache.clone().unwrap_or_default(),
            backing: context.db.clone_backing(),
        }
    }

    /// Returns whether a snapshot exists.
    pub fn has_snapshot(&self, id: U256) -> bool {
        self.snapshots.get(id).is_some()
    }

    /// Creates an EVM over the active context without copying its accepted state.
    ///
    /// A context that never ran is initialized first (network genesis, sentinels). Pass the EVM to
    /// [`Self::park`] to accept its state, or drop it to discard it.
    pub fn create_evm<'a>(&self) -> Evm<'a, F::Types> {
        let context = self.active();
        let db = context.db.clone_backing();
        let env = context.env;
        let factory = self.factory.configure_for(&db, &env);
        match &context.cache {
            None => factory.create_evm(db, env),
            Some(cache) => {
                let view = AcceptedView { cache: Arc::clone(cache), backing: db };
                let mut evm = factory.new_evm(view, env);
                *evm.ext_mut() = context.ext.clone();
                evm
            }
        }
    }

    /// Accepts `evm`'s state, environment and network extension state into the active context.
    ///
    /// Call this between transactions, with an EVM created by [`Self::create_evm`] whose
    /// executions were committed. In-flight state that was not committed is discarded, and a
    /// deferred environment change is applied first.
    pub fn park(&mut self, mut evm: Evm<'_, F::Types>) {
        self.begin_transaction(&mut evm);
        evm.state_mut().clear_transaction_state();
        let is_view = AcceptedView::is(evm.database());
        let local = mem::take(&mut evm.state_mut().overlay_db_mut().cache);
        let env = evm_env(&evm);
        let ext = evm.ext().clone();
        // Release the EVM's handle on the accepted state before updating it in place.
        drop(evm);
        let context = self.active_mut();
        match (is_view, &mut context.cache) {
            (true, Some(accepted)) => Arc::make_mut(accepted).merge(local),
            _ => context.cache = Some(Arc::new(local)),
        }
        context.env = env;
        context.ext = ext;
    }

    /// Materializes the accepted state `evm` reads through a shared view into its own overlay.
    ///
    /// Operations that capture or replace the running state call this first; it copies the
    /// accepted state once per execution.
    pub fn materialize(&self, evm: &mut Evm<'_, F::Types>) {
        if !AcceptedView::is(evm.database()) {
            return;
        }
        let context = self.active();
        let mut cache = context.cache.as_deref().cloned().unwrap_or_default();
        // Replace only the overlay's cache and backing database: `Evm::set_database` would also
        // clear the overlay and the in-flight transaction state.
        let overlay = evm.state_mut().overlay_db_mut();
        cache.merge(mem::take(&mut overlay.cache));
        overlay.cache = cache;
        overlay.db = Box::new(context.db.clone_backing());
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
        let suspended = self.suspend(evm);
        self.forks.active = id;
        Ok(self.resume(evm, suspended, phase))
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
        self.roll_fork_with(evm, id, db, env, None, phase)
    }

    /// Like [`Self::roll_fork`], but resumes the fork with `cache` as its accepted state, such as
    /// state replayed up to a transaction.
    pub fn roll_fork_with(
        &mut self,
        evm: &mut Evm<'_, F::Types>,
        id: Option<U256>,
        db: impl BackingDb,
        env: EvmEnv<F::Types>,
        cache: Option<Cache>,
        phase: ExecutionPhase,
    ) -> eyre::Result<EnvTransition> {
        let active = self.forks.active();
        let id = id.or(active);
        if id != active {
            self.replace_inactive(id, db, env, cache)?;
            return Ok(EnvTransition::Applied);
        }
        let ext = self.factory.configure_for(&db, &env).evm_ext(&env);
        let suspended = self.suspend(evm);
        let context = self.active_mut();
        *context = Parked::new(Box::new(db), env, ext);
        context.cache = cache.map(Arc::new);
        Ok(self.resume(evm, suspended, phase))
    }

    /// Moves the inactive fork `id`, or the local context for `None`, onto a new backing database
    /// and environment, optionally with `cache` as its accepted state.
    ///
    /// The fork's accepted state is discarded except for persistent accounts.
    pub fn replace_inactive(
        &mut self,
        id: Option<U256>,
        db: impl BackingDb,
        env: EvmEnv<F::Types>,
        cache: Option<Cache>,
    ) -> eyre::Result<()> {
        eyre::ensure!(id != self.forks.active(), "fork {} is active", id.unwrap_or_default());
        let Some(previous) = self.forks.context(id) else {
            eyre::bail!("fork {} does not exist", id.unwrap_or_default());
        };
        let previous = previous.cache.clone();
        let ext = self.factory.configure_for(&db, &env).evm_ext(&env);
        let mut context = Parked::new(Box::new(db), env, ext);
        if let Some(mut cache) = cache {
            if let Some(previous) = previous {
                self.forks.merge_persistent(&previous, &mut cache);
            }
            context.cache = Some(Arc::new(cache));
        }
        *self.forks.context_mut(id).expect("context exists") = context;
        Ok(())
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
    pub fn snapshot(&mut self, evm: &mut Evm<'_, F::Types>, extra: C) -> U256 {
        self.materialize(evm);
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
    /// Logs emitted since the snapshot are kept. Persistent accounts and forks created after the
    /// snapshot survive the revert; every fork that existed at the snapshot gets its state back.
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
        let mut current = mem::replace(&mut self.forks, forks);
        self.forks.next_id = current.next_id;
        self.forks.persistent = mem::take(&mut current.persistent);
        for (id, entry) in current.forks {
            self.forks.forks.entry(id).or_insert(entry);
        }
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
        self.nested_transaction_with(evm, tx, NestedTx::default())
    }

    /// Like [`Self::nested_transaction`], with an explicit environment, network extension state
    /// and inspector.
    pub fn nested_transaction_with(
        &self,
        evm: &mut Evm<'_, F::Types>,
        tx: &Recovered<<F::Types as EvmTypesHost>::Tx>,
        nested: NestedTx<'_, F::Types>,
    ) -> eyre::Result<TxResult<F::Types>> {
        let NestedTx { env, ext, inspector } = nested;
        let mut executor = self.nested_evm(evm, tx, env, ext);
        if let Some(inspector) = inspector {
            executor.set_inspector(inspector);
        }
        let executed = executor.transact(tx).map_err(|err| eyre::eyre!("{err:?}"))?;
        let result = executed.result().clone();
        if result.status {
            merge_nested_changes(evm.state_mut(), &executed)?;
        }
        Ok(result)
    }

    /// Builds the EVM a nested transaction of the running one executes on: a copy of the running
    /// transaction's current state, with `env` or the running environment, and `ext` or network
    /// extension state derived from the running EVM for `tx`.
    ///
    /// Merge a successful transaction's changes back with [`merge_nested_changes`]. Use this
    /// instead of [`Self::nested_transaction_with`] when the nested inspector needs access to the
    /// lifecycle's owner while it runs.
    pub fn nested_evm<'a>(
        &self,
        evm: &mut Evm<'_, F::Types>,
        tx: &Recovered<<F::Types as EvmTypesHost>::Tx>,
        env: Option<EvmEnv<F::Types>>,
        ext: Option<<F::Types as EvmTypesHost>::EvmExt>,
    ) -> Evm<'a, F::Types> {
        self.materialize(evm);
        let db = self.forks.active_db();
        let mut state = evm.state().snapshot().into_state(db.clone_backing());
        state.commit_transaction();
        state.clear_transaction_state();
        let ext = ext.unwrap_or_else(|| {
            let mut ext = evm.ext().clone();
            ext.for_transaction(tx);
            ext
        });
        let env = env.unwrap_or_else(|| evm_env(evm));
        let mut executor = self.factory.configure_for(&db, &env).new_evm(db, env);
        *executor.state_mut() = state;
        *executor.ext_mut() = ext;
        executor
    }

    /// Executes `message` as a nested frame of the transaction `evm` is running, such as for
    /// `deployCode`, with `inspector` observing it.
    ///
    /// The frame runs on a separate EVM that takes over the running state for its duration, so the
    /// running EVM's own inspector is not re-entered. Its writes, logs, block and network extension
    /// state are handed back to `evm`; a failed frame rolls its writes back like any other frame.
    pub fn nested_message(
        &self,
        evm: &mut Evm<'_, F::Types>,
        tx_env: &TxEnv<F::Types>,
        message: &mut Message<F::Types>,
        inspector: Option<&mut dyn Inspector<F::Types>>,
    ) -> Result<MessageResult<F::Types>, ExecutionError> {
        let env = evm_env(evm);
        let db = self.forks.active_db();
        let factory = self.factory.configure_for(&db, &env);
        let mut frame = factory.new_evm_with_ext(EmptyDB::default(), env, evm.ext().clone());
        *frame.state_mut() = mem::replace(evm.state_mut(), State::new(EmptyDB::default()));
        if let Some(inspector) = inspector {
            frame.set_inspector(inspector);
        }
        let result = Host::execute_message(&mut frame, tx_env, message);
        frame.clear_inspector();
        let state = mem::replace(frame.state_mut(), State::new(EmptyDB::default()));
        // SAFETY: `State`'s lifetime only bounds its backing database. The frame's state is the
        // running state moved in above, whose database outlives `evm`; the frame only ever
        // installs `'static` backing databases, such as on a fork switch.
        *evm.state_mut() = unsafe { mem::transmute::<State<'_>, State<'_>>(state) };
        *evm.ext_mut() = frame.ext().clone();
        evm.set_block(*frame.block());
        result
    }

    /// Executes `tx` on the inactive fork `id`, or the local context for `None`, and accepts its
    /// state there, such as for `vm.transact` on another fork.
    ///
    /// Persistent accounts carry the running transaction's state into the fork, and the
    /// transaction's changes to them are merged back into the running transaction as journaled
    /// writes.
    pub fn transact_on_inactive(
        &mut self,
        evm: &mut Evm<'_, F::Types>,
        id: Option<U256>,
        tx: &Recovered<<F::Types as EvmTypesHost>::Tx>,
        nested: NestedTx<'_, F::Types>,
    ) -> eyre::Result<TxResult<F::Types>> {
        eyre::ensure!(id != self.forks.active(), "fork {} is active", id.unwrap_or_default());
        let Some(context) = self.forks.context(id) else {
            eyre::bail!("fork {} does not exist", id.unwrap_or_default());
        };
        let db = context.db.clone_backing();
        let env = nested.env.unwrap_or(context.env);
        let ext = nested.ext.unwrap_or_else(|| {
            let mut ext = context.ext.clone();
            ext.for_transaction(tx);
            ext
        });
        let factory = self.factory.configure_for(&db, &env);
        // A context that never ran gets its network genesis before the transaction.
        let mut executor = match &context.cache {
            None => factory.create_evm(db, env),
            Some(cache) => {
                let mut executor = factory.new_evm(db, env);
                executor.state_mut().overlay_db_mut().cache = Cache::clone(cache);
                executor
            }
        };
        *executor.ext_mut() = ext;

        // Carry the running transaction's accepted and in-flight persistent accounts over.
        self.materialize(evm);
        let mut running = evm.state().snapshot().into_state(self.forks.active_db());
        running.commit_transaction();
        self.forks.merge_persistent(
            &running.overlay_db().cache,
            &mut executor.state_mut().overlay_db_mut().cache,
        );

        if let Some(inspector) = nested.inspector {
            executor.set_inspector(inspector);
        }
        let executed = executor.transact(tx).map_err(|err| eyre::eyre!("{err:?}"))?;
        let persistent = &self.forks.persistent;
        merge_changes(evm.state_mut(), &executed, |address| persistent.contains(address))?;
        let result = executed.commit();
        executor.clear_inspector();
        let cache = mem::take(&mut executor.state_mut().overlay_db_mut().cache);
        self.forks.context_mut(id).expect("context exists").cache = Some(Arc::new(cache));
        Ok(result)
    }

    /// Accepts the running state and stores it, with the running environment, in the active
    /// context. Returns what the running transaction carries into the next context.
    fn suspend(&mut self, evm: &mut Evm<'_, F::Types>) -> Suspended {
        self.materialize(evm);
        let state = evm.state_mut();
        state.commit_transaction();
        let logs = mem::take(state.logs_mut());
        let prewarm = state.prewarm_set().clone();
        let cache = state.overlay_db().cache.clone();
        let env = evm_env(evm);
        let ext = evm.ext().clone();
        let context = self.active_mut();
        context.cache = Some(Arc::new(cache.clone()));
        context.env = env;
        context.ext = ext;
        Suspended { cache, logs, prewarm }
    }

    /// Installs the active context into `evm`, carrying persistent accounts, logs and base warmth
    /// over from the suspended context.
    fn resume(
        &mut self,
        evm: &mut Evm<'_, F::Types>,
        suspended: Suspended,
        phase: ExecutionPhase,
    ) -> EnvTransition {
        let Suspended { cache: source, logs, prewarm } = suspended;
        let context = self.active_mut();
        let fresh = context.cache.is_none();
        // The running EVM owns the active context's accepted state.
        let mut cache = context.cache.take().map(Arc::unwrap_or_clone).unwrap_or_default();
        let db = context.db.clone_backing();
        let env = context.env;
        let ext = context.ext.clone();
        self.forks.merge_persistent(&source, &mut cache);

        let mut state = State::new(db.clone_backing());
        state.overlay_db_mut().cache = cache;
        *state.logs_mut() = logs;
        state.set_prewarm_set(prewarm);
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

/// What a running transaction carries from a suspended context into the next one.
struct Suspended {
    /// The suspended context's accepted state, the source of persistent accounts.
    cache: Cache,
    /// Logs of the running transaction.
    logs: Vec<Log>,
    /// Base warmth of the running transaction, such as its access list.
    prewarm: PrewarmSet,
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

/// Applies a successful nested transaction's changes to the running transaction's `state` as
/// journaled writes, so they revert with the enclosing call.
pub fn merge_nested_changes<T: EvmTypesHost>(
    state: &mut State<'_>,
    executed: &ExecutedTx<'_, '_, T>,
) -> DbResult<()> {
    merge_changes(state, executed, |_| true)
}

/// Applies a nested transaction's changes to the accounts selected by `filter` to `state` as
/// journaled writes.
fn merge_changes<T: EvmTypesHost>(
    state: &mut State<'_>,
    executed: &ExecutedTx<'_, '_, T>,
    filter: impl Fn(&Address) -> bool,
) -> DbResult<()> {
    for change in executed.account_changes().filter(|change| change.is_changed()) {
        let address = change.address;
        if !filter(&address) {
            continue;
        }
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
