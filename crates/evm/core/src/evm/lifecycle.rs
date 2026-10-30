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
//! once and record the rest as the pending environment ([`ExecutionLifecycle::pending_env`]);
//! [`ExecutionLifecycle::begin_transaction`] then applies it at the next transaction boundary.
//! Until then, the inspector stack runs frames started after the change on a frame EVM built with
//! the pending environment ([`ExecutionLifecycle::begin_frame_evm`]).
//!
//! Suspending a context accepts its in-flight writes, so a fork switch is not undone when an
//! enclosing call reverts.

use super::{
    AcceptedView, BackingDb, EvmEnv, ForkInfo, ForkPosition, ForkRegistry, FoundryEvmExt,
    FoundryEvmFactory, LifecycleSnapshot, Parked, StoredExt, evm_env,
};
use crate::{
    backend::{RevertStateSnapshotAction, transaction_error},
    state_snapshot::StateSnapshots,
};
use alloy_consensus::{Typed2718, transaction::Recovered};
use alloy_eips::eip2930::AccessList;
use alloy_primitives::{
    Address, B256, KECCAK256_EMPTY, Log, U256,
    map::{AddressMap, AddressSet, Entry, HashMap, HashSet},
};
use evm2::{
    Evm, EvmFeatures, EvmTypes, EvmTypesHost, ExecutedTx, Inspector, TxResult, TxResultWithState,
    bytecode::Bytecode,
    evm::{
        AccountChanges, AccountFlags, AccountHandle, AccountInfo, Cache, CacheDB, DbResult,
        DynDatabase, EmptyDB, JournalEntry, PendingState, PrewarmSet, State, StateCheckpoint,
        StateSnapshot,
    },
};
use std::{iter, mem, sync::Arc};

/// Whether a lifecycle operation runs between transactions or inside one, such as from a
/// cheatcode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionPhase {
    /// Between transactions: every part of the environment can change.
    Boundary,
    /// Inside a running transaction.
    InFlight,
}

/// A transaction prepared to execute as a nested transaction of the running one, such as for
/// `vm.transact`, whose inspector may hold the lifecycle's owner while it runs.
///
/// Build it with [`ExecutionLifecycle::prepare_nested`] or
/// [`ExecutionLifecycle::prepare_on_inactive`], run it with [`Self::execute`] and store its
/// outcome with [`ExecutionLifecycle::store_executed`].
pub struct PreparedTx<T: EvmTypesHost> {
    executor: Evm<'static, T>,
    tx: Recovered<T::Tx>,
    /// The inactive context the transaction executes on, with the persistent accounts whose
    /// changes merge back into the running transaction.
    inactive: Option<(Option<U256>, AddressSet)>,
    /// The fork whose canonical position and block cursor advance once the transaction executed.
    cursor: Option<ForkCursor<T>>,
    /// The transaction's EIP-2930 access list, if its caller recorded it.
    access_list: Option<AccessList>,
}

/// A fork's canonical position and the network state retaining its block cursor.
pub type ForkCursor<T> = (U256, ForkPosition, <T as EvmTypesHost>::EvmExt);

impl<T: EvmTypes<Tx: Typed2718>> PreparedTx<T> {
    /// Returns the transaction.
    pub const fn tx(&self) -> &Recovered<T::Tx> {
        &self.tx
    }

    /// Advances fork `id` to `position`, with the network state retaining its block `cursor`,
    /// once the transaction executed.
    pub fn set_cursor(&mut self, id: U256, position: ForkPosition, cursor: T::EvmExt) {
        self.cursor = Some((id, position, cursor));
    }

    /// Returns the environment the transaction executes in.
    pub const fn env(&self) -> EvmEnv<T> {
        evm_env(&self.executor)
    }

    /// Returns the transaction's EIP-2930 access list, if recorded.
    pub const fn access_list(&self) -> Option<&AccessList> {
        self.access_list.as_ref()
    }

    /// Records the transaction's EIP-2930 access list.
    pub fn set_access_list(&mut self, access_list: Option<AccessList>) {
        self.access_list = access_list;
    }

    /// Executes the transaction observed by `inspector` and merges its changes into the running
    /// transaction of `evm` as journaled writes: all of them on the running transaction's
    /// context, where a failed transaction only keeps its nonce increment and fee payment, and
    /// the changes to persistent accounts on an inactive context, which accepts the transaction
    /// and whose resulting state the running transaction's other loaded accounts then take.
    /// With `untracked`, changes on the active context survive reverts of the enclosing calls.
    pub fn execute(
        self,
        evm: &mut Evm<'_, T>,
        inspector: Option<&mut dyn Inspector<T>>,
        untracked: bool,
    ) -> eyre::Result<ExecutedNested<T>> {
        let Self { executor, tx, inactive, cursor, access_list: _ } = self;
        let mut executor: Evm<'_, T> = executor;
        if let Some(inspector) = inspector {
            executor.set_inspector(inspector);
        }
        let executed = executor.transact(&tx).map_err(transaction_error)?;
        let Some((id, persistent)) = inactive else {
            if untracked {
                // As on master, which committed the transaction to the database, accounts and
                // slots the running transaction has not loaded start from its writes.
                let unloaded = unloaded_changes(evm.state(), executed.account_changes());
                evm.state_mut().commit_source(&unloaded);
            }
            let mut persistent = Vec::new();
            merge_changes(
                evm.state_mut(),
                executed.account_changes(),
                |_| true,
                untracked.then_some(&mut persistent),
                None,
            )?;
            let result = executed.result().clone();
            return Ok(ExecutedNested { result, context: None, cursor, persistent });
        };
        // The inactive context's code is not in the running transaction's database.
        let TxResultWithState { result, pending_state, .. } = executed.detach();
        let source = executor.state_mut().overlay_db_mut();
        merge_changes(
            evm.state_mut(),
            pending_state.account_changes(),
            |address| persistent.contains(address),
            None,
            Some(source),
        )?;
        source.commit_pending(&pending_state);
        rebase_loaded_accounts(evm.state_mut(), executor.state_mut(), &persistent)?;
        executor.clear_inspector();
        let cache = mem::take(&mut executor.state_mut().overlay_db_mut().cache);
        Ok(ExecutedNested { result, context: Some((id, cache)), cursor, persistent: Vec::new() })
    }
}

/// The outcome of a [`PreparedTx`], to store with [`ExecutionLifecycle::store_executed`].
#[must_use = "an inactive context's accepted state must be stored"]
pub struct ExecutedNested<T: EvmTypesHost> {
    result: TxResult<T>,
    /// The inactive context the transaction was accepted on, with its new accepted state.
    context: Option<(Option<U256>, Cache)>,
    /// See [`PreparedTx::set_cursor`].
    cursor: Option<ForkCursor<T>>,
    /// Writes of an untracked merge, to replay when an enclosing call reverts.
    persistent: Vec<PersistentWrite>,
}

impl<T: EvmTypesHost> ExecutedNested<T> {
    /// Takes the fork cursor to advance, see [`PreparedTx::set_cursor`].
    pub const fn take_cursor(&mut self) -> Option<ForkCursor<T>> {
        self.cursor.take()
    }

    /// Takes the writes of an untracked merge, see [`PersistentWrite`].
    pub fn take_persistent(&mut self) -> Vec<PersistentWrite> {
        mem::take(&mut self.persistent)
    }
}

/// A write that must survive reverts of the enclosing calls, such as a cheatcode's state edit. It
/// is applied as a journaled write, and the owner replays it after each enclosing call that
/// reverts. Balance and nonce writes replay as deltas, so they compose with value transfers and
/// nonce bumps undone by the revert.
#[derive(Clone, Debug)]
pub enum PersistentWrite {
    /// Adds a wrapping delta to an account's balance.
    Balance(Address, U256),
    /// Adds a delta to an account's nonce, saturating at its bounds.
    Nonce {
        /// The account.
        address: Address,
        /// The change to the nonce.
        delta: i128,
        /// Whether the running transaction had created the account when the write was made. A
        /// revert that undoes the creation also undoes the write.
        created: bool,
    },
    /// Sets an account's code.
    Code(Address, B256, Bytecode),
    /// Wipes an account's storage.
    Wipe(Address),
    /// Sets a storage slot.
    Slot(Address, U256, U256),
    /// Marks an account self-destructed, so the running transaction destroys it when it ends.
    Destruct(Address),
}

/// The state a nested transaction left an existing account in before the transaction ended and
/// destroyed it, which the running transaction keeps until it ends itself, as on master.
#[derive(Clone, Debug)]
pub struct DestructedAccount {
    info: AccountInfo,
    code: Bytecode,
    slots: Vec<(U256, U256)>,
}

/// Returns the accounts the running transaction in `state` self-destructed without creating them,
/// with their state before the transaction ends and destroys them, such as at the end of the root
/// frame of `executeTransaction`'s transaction.
pub fn destructed_accounts(state: &mut State<'_>) -> DbResult<AddressMap<DestructedAccount>> {
    let loaded = state
        .loaded_accounts()
        .map(|(address, slots)| (*address, slots.copied().collect::<Vec<_>>()))
        .collect::<Vec<_>>();
    let mut destructed = AddressMap::default();
    for (address, keys) in loaded {
        let code = {
            let mut account = state.account(&address)?;
            if !account.is_destructed() || account.is_created() {
                continue;
            }
            account.load_code()?
        };
        let Some(info) = state.account_info_untracked(&address)? else { continue };
        let slots = keys
            .into_iter()
            .map(|key| Ok((key, state.storage_slot_untracked(&address, &key)?)))
            .collect::<DbResult<_>>()?;
        destructed.insert(address, DestructedAccount { info, code, slots });
    }
    Ok(destructed)
}

impl PersistentWrite {
    /// Returns the nonce write that `account`, whose nonce was `previous`, just made.
    pub fn nonce(account: &AccountHandle<'_, '_>, previous: u64) -> Self {
        Self::Nonce {
            address: account.address(),
            delta: i128::from(account.nonce()) - i128::from(previous),
            created: account.is_created(),
        }
    }

    /// Applies the write to `state` as a journaled write.
    pub fn apply(&self, state: &mut State<'_>) -> DbResult<()> {
        match self {
            Self::Balance(address, delta) => {
                let mut account = state.account(address)?;
                let balance = account.balance().wrapping_add(*delta);
                account.set_balance(balance);
            }
            Self::Nonce { address, delta, created } => {
                let mut account = state.account(address)?;
                if *created && !account.is_created() {
                    return Ok(());
                }
                account.touch();
                let nonce = (i128::from(account.nonce()) + delta).max(0);
                account.set_nonce(u64::try_from(nonce).unwrap_or(u64::MAX));
            }
            Self::Code(address, code_hash, code) => {
                state.account(address)?.set_code(*code_hash, code.clone());
            }
            Self::Wipe(address) => state.storage(address)?.wipe(),
            Self::Slot(address, key, value) => state.storage_slot(address, *key)?.set(*value),
            Self::Destruct(address) => state.account(address)?.mark_destructed(),
        }
        Ok(())
    }

    /// Applies `writes` in order like [`Self::apply`], except that the nonce deltas of each
    /// account are summed and saturated once, as REVM's undo of the reverted nonce changes was.
    pub fn apply_all<'w>(
        writes: impl IntoIterator<Item = &'w Self>,
        state: &mut State<'_>,
    ) -> DbResult<()> {
        let mut nonces = AddressMap::<i128>::default();
        for write in writes {
            match write {
                Self::Nonce { address, delta, created } => {
                    if !*created || state.account(address)?.is_created() {
                        *nonces.entry(*address).or_default() += delta;
                    }
                }
                write => write.apply(state)?,
            }
        }
        for (address, delta) in nonces {
            Self::Nonce { address, delta, created: false }.apply(state)?;
        }
        Ok(())
    }
}

/// Fork, snapshot and nested-execution lifecycle for one EVM network family.
pub struct ExecutionLifecycle<F: FoundryEvmFactory> {
    factory: F,
    forks: ForkRegistry<F::Types>,
    snapshots: StateSnapshots<LifecycleSnapshot<F::Types>>,
    pending_env: Option<EvmEnv<F::Types>>,
    /// Base warm set of the running transaction from before a fork switch carried warmth into it.
    tx_prewarm: Option<PrewarmSet>,
    /// Transaction-start info of the persistent accounts a fork switch accepted the running
    /// transaction's changes to.
    tx_original: AddressMap<Option<AccountInfo>>,
    /// Warm slots of persistent accounts the last fork switch carried into the next context.
    switch_slots: AddressMap<HashSet<U256>>,
}

impl<F: FoundryEvmFactory> Clone for ExecutionLifecycle<F> {
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
            forks: self.forks.clone(),
            snapshots: self.snapshots.clone(),
            pending_env: self.pending_env,
            tx_prewarm: self.tx_prewarm.clone(),
            tx_original: self.tx_original.clone(),
            switch_slots: self.switch_slots.clone(),
        }
    }
}

impl<F: FoundryEvmFactory> std::fmt::Debug for ExecutionLifecycle<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionLifecycle").field("forks", &self.forks).finish_non_exhaustive()
    }
}

impl<F: FoundryEvmFactory> ExecutionLifecycle<F> {
    /// Creates a lifecycle whose active context is local execution over `db`.
    pub fn new(factory: F, db: impl BackingDb, env: EvmEnv<F::Types>) -> Self {
        let ext = factory.configure_for(&db, &env).evm_ext(&env);
        Self {
            factory,
            forks: ForkRegistry::new(Parked::new(Box::new(db), env, ext)),
            snapshots: StateSnapshots::default(),
            pending_env: None,
            tx_prewarm: None,
            tx_original: AddressMap::default(),
            switch_slots: AddressMap::default(),
        }
    }

    /// Returns the factory.
    pub const fn factory(&self) -> &F {
        &self.factory
    }

    /// Replaces the factory used for EVMs built from now on.
    pub fn set_factory(&mut self, factory: F) {
        self.factory = factory;
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

    /// Records `value` as the accepted value of a slot the running transaction restored the
    /// original of when it resumed the active context, such as after refreshing the slot from the
    /// remote, so that a revert leaving the context keeps it.
    pub fn refresh_switched_slot(&mut self, address: &Address, slot: U256, value: U256) {
        if let Some(switched) = self.active_mut().switched.get_mut(address)
            && let Some(accepted) = switched.get_mut(&slot)
        {
            *accepted = value;
        }
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
    pub fn accepted_db(&self) -> impl BackingDb + use<F> {
        self.active().accepted_db()
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
                *evm.ext_mut() = context.ext.get();
                evm
            }
        }
    }

    /// Accepts `evm`'s state, environment and network extension state into the active context. A
    /// fork keeps its own chain ID.
    ///
    /// Call this between transactions, with an EVM created by [`Self::create_evm`] whose
    /// executions were committed. In-flight state that was not committed is discarded, and a
    /// deferred environment change is applied first.
    pub fn park(&mut self, evm: Evm<'_, F::Types>) {
        self.accept(evm, true);
    }

    /// Like [`Self::park`], for an EVM whose executions were not committed, such as a call's: its
    /// pending writes are discarded, but the accepted state it carries, such as after a fork
    /// switch, is kept.
    pub(crate) fn park_detached(&mut self, evm: Evm<'_, F::Types>) {
        self.accept(evm, false);
    }

    /// Accepts `evm`'s state, environment and network extension state into the active context.
    ///
    /// An EVM that materialized its accepted state holds the context's whole accepted state, which
    /// replaces the context's. One that still reads it through a shared view only holds what its
    /// executions read and, if `committed`, wrote, which is merged into the context's; without
    /// committed writes it only caches reads, so it is dropped rather than copying the shared
    /// accepted state.
    fn accept(&mut self, mut evm: Evm<'_, F::Types>, committed: bool) {
        self.begin_transaction(&mut evm);
        evm.state_mut().clear_transaction_state();
        let is_view = AcceptedView::is(evm.database());
        let local = mem::take(&mut evm.state_mut().overlay_db_mut().cache);
        let env = evm_env(&evm);
        let mut ext = evm.ext().clone();
        // Permission to replay protocol history belongs to one transaction, not to the context.
        ext.set_protocol_replay(false);
        // Release the EVM's handle on the accepted state before updating it in place.
        drop(evm);
        let context = self.active_mut();
        match (is_view, &mut context.cache) {
            (true, Some(accepted)) => {
                if committed {
                    Arc::make_mut(accepted).merge(local);
                }
            }
            _ => context.cache = Some(Arc::new(local)),
        }
        context.ext = StoredExt::new(&ext);
        // Like REVM's per-fork journals, inactive contexts keep what they loaded and created until
        // they finish a transaction themselves, and the local context keeps seeding forks.
        context.loaded.clear();
        context.created.clear();
        self.store_env(env);
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
        self.sync_database_policy(evm);
    }

    /// Registers a fork without selecting it and returns its local fork ID.
    pub fn create_fork(
        &mut self,
        info: ForkInfo,
        db: impl BackingDb,
        env: EvmEnv<F::Types>,
    ) -> U256 {
        let ext = self.factory.configure_for(&db, &env).evm_ext(&env);
        let mut context = Parked::new(Box::new(db), env, ext);
        context.inherit_local(&self.forks.local);
        self.forks.insert(info, context)
    }

    /// Makes `id` the running context, or the local context for `None`.
    ///
    /// Persistent accounts carry their accepted state into the target. Leaving the local context
    /// makes what the running transaction loaded and created there the initial state of every
    /// fork, as REVM's initial fork journal did.
    pub fn select_fork(
        &mut self,
        evm: &mut Evm<'_, F::Types>,
        id: Option<U256>,
        phase: ExecutionPhase,
    ) -> eyre::Result<()> {
        if self.forks.context(id).is_none() {
            eyre::bail!("fork {} does not exist", id.unwrap_or_default());
        }
        if id == self.forks.active() {
            return Ok(());
        }
        let suspended = self.suspend(evm);
        if self.forks.active.is_none() {
            let ForkRegistry { local, forks, .. } = &mut self.forks;
            for entry in forks.values_mut() {
                entry.context.inherit_local(local);
            }
        }
        self.forks.active = id;
        self.resume(evm, suspended, false, phase);
        Ok(())
    }

    /// Makes `id` the active context, or the local context for `None`, without suspending the
    /// running state: for a throwaway copy of the lifecycle whose transaction already runs over
    /// `id`'s state, such as one prepared with [`Self::prepare_on_inactive`].
    pub const fn activate_detached(&mut self, id: Option<U256>) {
        self.forks.active = id;
    }

    /// Moves fork `id`, or the running context for `None`, onto a new backing database and
    /// environment, such as a later block.
    ///
    /// The fork starts over from the initial fork state, keeping persistent accounts and accounts
    /// created on the fork. Rolling the running context also keeps other loaded accounts, but
    /// reloads only their info from the new database, discarding their pre-roll storage slots.
    ///
    /// With `cache`, the fork resumes with it as its accepted state, such as state replayed up to a
    /// transaction.
    pub fn roll_fork(
        &mut self,
        evm: &mut Evm<'_, F::Types>,
        id: Option<U256>,
        db: impl BackingDb,
        env: EvmEnv<F::Types>,
        cache: Option<Cache>,
        phase: ExecutionPhase,
    ) -> eyre::Result<()> {
        let active = self.forks.active();
        let id = id.or(active);
        if id != active {
            return self.replace_inactive(id, db, env, cache);
        }
        let ext = self.factory.configure_for(&db, &env).evm_ext(&env);
        let mut suspended = self.suspend(evm);
        for (address, slots) in &mut suspended.loaded {
            if !suspended.created.contains(address) && !self.forks.is_persistent(address) {
                slots.clear();
            }
        }
        let mut context = Parked::new(Box::new(db), env, ext);
        context.cache = cache.map(Arc::new);
        context.inherit_local(&self.forks.local);
        *self.active_mut() = context;
        self.resume(evm, suspended, true, phase);
        Ok(())
    }

    /// Moves the inactive fork `id`, or the local context for `None`, onto a new backing database
    /// and environment, optionally with `cache` as its accepted state.
    ///
    /// The fork's accepted state is discarded except for persistent accounts and accounts created
    /// on it, and the fork starts over from the initial fork state, as in REVM.
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
        let ext = self.factory.configure_for(&db, &env).evm_ext(&env);
        let mut context = Parked::new(Box::new(db), env, ext);
        if let Some(mut cache) = cache {
            if let Some(previous_cache) = &previous.cache {
                self.forks.merge_persistent(previous_cache, &mut cache);
                self.forks.merge_persistent_loaded(&previous.loaded, &mut context.loaded);
            }
            context.cache = Some(Arc::new(cache));
        }
        if id.is_some() {
            context.inherit_local(&self.forks.local);
        }
        if let Some(previous_cache) = &previous.cache {
            let loaded = previous
                .loaded
                .iter()
                .filter(|(address, _)| previous.created.contains(*address))
                .map(|(address, slots)| (*address, slots.clone()))
                .collect();
            context.inherit(&loaded, &previous.created, previous_cache);
        }
        *self.forks.context_mut(id).expect("context exists") = context;
        Ok(())
    }

    /// Returns the environment change deferred by an in-flight operation, applied at the next
    /// transaction.
    pub const fn pending_env(&self) -> Option<&EvmEnv<F::Types>> {
        self.pending_env.as_ref()
    }

    /// Returns the base warm set of the running transaction if a fork switch carried warmth into
    /// its warm set, which cooling the transaction's state must restore.
    pub const fn tx_prewarm(&self) -> Option<&PrewarmSet> {
        self.tx_prewarm.as_ref()
    }

    /// Removes `address` from the running transaction's base warmth in `state` and in the
    /// warmth that fork switches and isolated static calls restore, so it stays cold until it is
    /// accessed again.
    pub fn remove_prewarm(&mut self, state: &mut State<'_>, address: &Address) {
        state.prewarm_set_mut().remove(address);
        if let Some(prewarm) = &mut self.tx_prewarm {
            prewarm.remove(address);
        }
        // Like REVM's fork switches, which copy the journaled state of persistent accounts into
        // the selected fork, a persistent account resumes cold in every context.
        if self.forks.is_persistent(address) {
            self.forks.local.warm.remove(address);
            for entry in self.forks.forks.values_mut() {
                entry.context.warm.remove(address);
            }
        } else {
            self.active_mut().warm.remove(address);
        }
    }

    /// Returns whether [`Self::remove_prewarm`] would change the lifecycle, not only `state`.
    pub fn tracks_prewarm(&self, address: &Address) -> bool {
        self.tx_prewarm.as_ref().is_some_and(|prewarm| prewarm.is_warm(address))
            || iter::once(&self.forks.local)
                .chain(self.forks.forks.values().map(|entry| &entry.context))
                .any(|context| context.warm.contains_key(address))
    }

    /// Returns whether any context keeps warmth of a persistent account for the running
    /// transaction, which [`Self::take_persistent_warmth`] would take.
    pub fn has_persistent_warmth(&self) -> bool {
        let ForkRegistry { local, forks, persistent, .. } = &self.forks;
        iter::once(local)
            .chain(forks.values().map(|entry| &entry.context))
            .any(|context| context.warm.keys().any(|address| persistent.contains(address)))
    }

    /// Takes the warmth of persistent accounts that contexts kept for the running transaction, so
    /// that a nested transaction, such as an isolated call, resumes them cold in every context as
    /// REVM's fresh journal did. Hand it back with [`Self::restore_persistent_warmth`] when the
    /// nested transaction ends.
    pub fn take_persistent_warmth(&mut self) -> Vec<(Option<U256>, AddressMap<HashSet<U256>>)> {
        let ForkRegistry { local, forks, persistent, .. } = &mut self.forks;
        let forks = forks.iter_mut().map(|(id, entry)| (Some(*id), &mut entry.context));
        iter::once((None, local))
            .chain(forks)
            .map(|(id, context)| {
                let (taken, kept) = mem::take(&mut context.warm)
                    .into_iter()
                    .partition::<AddressMap<_>, _>(|(address, _)| persistent.contains(address));
                context.warm = kept;
                (id, taken)
            })
            .collect()
    }

    /// Hands persistent warmth taken with [`Self::take_persistent_warmth`] back to its contexts.
    pub fn restore_persistent_warmth(
        &mut self,
        warmth: Vec<(Option<U256>, AddressMap<HashSet<U256>>)>,
    ) {
        for (id, warm) in warmth {
            if let Some(context) = self.forks.context_mut(id) {
                for (address, slots) in warm {
                    context.warm.entry(address).or_default().extend(slots);
                }
            }
        }
    }

    /// Applies an environment change deferred by an in-flight operation and forgets the warmth
    /// contexts kept from the previous transaction. Call before every transaction.
    pub fn begin_transaction(&mut self, evm: &mut Evm<'_, F::Types>) {
        self.tx_prewarm = None;
        self.tx_original.clear();
        self.forks.local.warm.clear();
        self.forks.local.originals.clear();
        self.forks.local.switched.clear();
        for entry in self.forks.forks.values_mut() {
            entry.context.warm.clear();
            entry.context.originals.clear();
            entry.context.switched.clear();
        }
        if let Some(mut env) = self.pending_env.take() {
            env.block = *evm.block();
            let db = self.forks.active_db();
            self.factory.configure_for(&db, &env).set_env(evm, env);
            self.sync_database_policy(evm);
        }
    }

    /// Switches `evm` to `env`. The block applies at once; a spec or chain ID change applies at
    /// once between transactions and is deferred to the next transaction while one is in flight.
    pub fn set_env(
        &mut self,
        evm: &mut Evm<'_, F::Types>,
        env: EvmEnv<F::Types>,
        phase: ExecutionPhase,
    ) {
        let db = self.forks.active_db();
        let factory = self.factory.configure_for(&db, &env);
        self.apply_env(&factory, evm, env, phase)
    }

    /// Captures the running context and every fork, and returns the snapshot ID.
    pub fn snapshot(&mut self, evm: &mut Evm<'_, F::Types>) -> U256 {
        self.materialize(evm);
        self.snapshots.insert(LifecycleSnapshot {
            state: evm.state().snapshot(),
            forks: self.forks.clone(),
            env: evm_env(evm),
            ext: StoredExt::new(evm.ext()),
        })
    }

    /// Restores snapshot `id` and returns the running state it replaced, or `None` if it does not
    /// exist.
    ///
    /// Logs emitted since the snapshot are kept. As on master, only the context that was active at
    /// the snapshot gets its state back and becomes active again; other contexts, persistent
    /// accounts and forks created after the snapshot are left as they are.
    /// The replaced state is materialized, so that a failing frame can reinstate it over a backing
    /// database with [`SnapshotRestores`](super::SnapshotRestores).
    pub fn revert(
        &mut self,
        evm: &mut Evm<'_, F::Types>,
        id: U256,
        action: RevertStateSnapshotAction,
        phase: ExecutionPhase,
    ) -> Option<StateSnapshot> {
        let snapshot = if action.is_keep() {
            self.snapshots.get(id).cloned()
        } else {
            self.snapshots.remove(id)
        }?;
        self.materialize(evm);
        let LifecycleSnapshot { state, mut forks, env, ext } = snapshot;
        let (replaced, logs) = if forks.active == self.forks.active {
            (None, evm.state().logs().to_vec())
        } else {
            // Like master, other contexts are not part of the snapshot. The running context keeps
            // what reached its state before the running transaction and drops the transaction's
            // changes, as master drops the journal of the fork active at the revert.
            let replaced = evm.state().snapshot();
            let state = evm.state_mut();
            let logs = mem::take(state.logs_mut());
            state.clear_transaction_state();
            let mut cache = mem::take(&mut state.overlay_db_mut().cache);
            // Values accepted at the switch into the context stay accepted, as before their
            // originals were restored.
            for (address, slots) in mem::take(&mut self.active_mut().switched) {
                cache.storage.entry(address).or_default().slots.extend(slots);
            }
            self.active_mut().cache = Some(Arc::new(cache));
            (Some(replaced), logs)
        };
        match forks.active {
            None => self.forks.local = forks.local,
            Some(id) => {
                let entry = forks.forks.remove(&id).expect("snapshot's active fork exists");
                self.forks.forks.insert(id, entry);
            }
        }
        self.forks.active = forks.active;
        let db = self.forks.active_db();
        let mut state = state.into_state(db.clone_backing());
        *state.logs_mut() = logs;
        let running = mem::replace(evm.state_mut(), state);
        let replaced = replaced.unwrap_or_else(|| running.snapshot());
        evm.ext_mut().replace_while_executing(ext.into_inner());
        let factory = self.factory.configure_for(&db, &env);
        self.apply_env(&factory, evm, env, phase);
        Some(replaced)
    }

    /// Deletes a snapshot, returning whether it existed.
    pub fn delete_snapshot(&mut self, id: U256) -> bool {
        self.snapshots.remove_at(id).is_some()
    }

    /// Deletes every snapshot.
    pub fn delete_snapshots(&mut self) {
        self.snapshots.clear();
    }

    /// Builds the EVM a nested transaction of the running one executes on: a copy of the running
    /// transaction's current state, with `env` or the running environment, and `ext` or network
    /// extension state derived from the running EVM for `tx`.
    pub fn nested_evm<'a>(
        &self,
        evm: &mut Evm<'_, F::Types>,
        tx: &Recovered<<F::Types as EvmTypesHost>::Tx>,
        env: Option<EvmEnv<F::Types>>,
        ext: Option<<F::Types as EvmTypesHost>::EvmExt>,
    ) -> Evm<'a, F::Types> {
        self.materialize(evm);
        let db = self.forks.active_db();
        // Copy the running state without its journal, so each nested transaction is independent
        // of how much the enclosing one already did.
        let state = evm.state().committed_copy(db.clone_backing());
        let ext = ext.unwrap_or_else(|| {
            let mut ext = evm.ext().clone();
            ext.for_next_transaction(tx);
            ext
        });
        let env = env.unwrap_or_else(|| evm_env(evm));
        let mut executor = self.factory.configure_for(&db, &env).new_evm(db, env);
        *executor.state_mut() = state;
        *executor.ext_mut() = ext;
        self.sync_database_policy(&mut executor);
        executor
    }

    /// Moves the transaction `evm` is running onto a separate frame EVM executing with `env`,
    /// such as for `deployCode` or an in-flight spec change.
    ///
    /// Execute messages on the returned EVM with [`Host::execute_message`] and hand its state back
    /// with [`Self::end_frame_evm`]; `evm` must not execute in between.
    ///
    /// # Safety
    ///
    /// The running state of `evm` moves into the returned EVM, whose lifetime `'a` the caller
    /// chooses. Every database in that state must outlive `'a`: it must be one the lifecycle
    /// installed, all of which are owned (`'static`), not a database borrowed by the caller.
    ///
    /// [`Host::execute_message`]: evm2::interpreter::Host::execute_message
    pub unsafe fn begin_frame_evm<'a>(
        &self,
        evm: &mut Evm<'_, F::Types>,
        env: EvmEnv<F::Types>,
    ) -> Evm<'a, F::Types> {
        let db = self.forks.active_db();
        let factory = self.factory.configure_for(&db, &env);
        let mut frame = factory.new_evm_with_ext(EmptyDB::default(), env, evm.ext().clone());
        let state = mem::replace(evm.state_mut(), State::new(EmptyDB::default()));
        // SAFETY: `State`'s lifetime only bounds its databases, which the caller guarantees
        // outlive `'a`.
        *frame.state_mut() = unsafe { mem::transmute::<State<'_>, State<'a>>(state) };
        *frame.tx_env_mut() = mem::take(evm.tx_env_mut());
        self.sync_database_policy(&mut frame);
        frame
    }

    /// Hands the state, transaction environment, block and network extension state of a frame EVM
    /// created with
    /// [`Self::begin_frame_evm`] back to `evm`, restoring `evm`'s empty-account policy on the
    /// active backing database.
    ///
    /// # Safety
    ///
    /// The running state of `frame` moves into `evm`. Every database in that state must outlive
    /// `evm`: it must be one the lifecycle installed, as for [`Self::begin_frame_evm`].
    pub unsafe fn end_frame_evm(&self, evm: &mut Evm<'_, F::Types>, mut frame: Evm<'_, F::Types>) {
        frame.clear_inspector();
        let state = mem::replace(frame.state_mut(), State::new(EmptyDB::default()));
        // SAFETY: `State`'s lifetime only bounds its databases, which the caller guarantees
        // outlive `evm`.
        *evm.state_mut() = unsafe { mem::transmute::<State<'_>, State<'_>>(state) };
        *evm.tx_env_mut() = mem::take(frame.tx_env_mut());
        *evm.ext_mut() = frame.ext().clone();
        evm.set_block(*frame.block());
        self.sync_database_policy(evm);
    }

    /// Prepares `tx` to execute as a nested transaction of the running one on the EVM
    /// [`Self::nested_evm`] builds.
    pub fn prepare_nested(
        &self,
        evm: &mut Evm<'_, F::Types>,
        tx: Recovered<<F::Types as EvmTypesHost>::Tx>,
        env: Option<EvmEnv<F::Types>>,
        ext: Option<<F::Types as EvmTypesHost>::EvmExt>,
    ) -> PreparedTx<F::Types> {
        PreparedTx {
            executor: self.nested_evm(evm, &tx, env, ext),
            tx,
            inactive: None,
            cursor: None,
            access_list: None,
        }
    }

    /// Prepares `tx` to execute on the inactive fork `id`, or the local context for `None`, and
    /// be accepted there, such as for `vm.transact` on another fork.
    ///
    /// Persistent accounts carry the running transaction's state into the fork, and the
    /// transaction's changes to them are merged back into the running transaction as journaled
    /// writes. The running transaction's other loaded accounts then take the fork's resulting
    /// state, see [`Self::rebase_onto_inactive`].
    pub fn prepare_on_inactive(
        &self,
        evm: &mut Evm<'_, F::Types>,
        id: Option<U256>,
        tx: Recovered<<F::Types as EvmTypesHost>::Tx>,
        env: Option<EvmEnv<F::Types>>,
        ext: Option<<F::Types as EvmTypesHost>::EvmExt>,
    ) -> eyre::Result<PreparedTx<F::Types>> {
        eyre::ensure!(id != self.forks.active(), "fork {} is active", id.unwrap_or_default());
        let Some(context) = self.forks.context(id) else {
            eyre::bail!("fork {} does not exist", id.unwrap_or_default());
        };
        let db = context.db.clone_backing();
        let env = env.unwrap_or(context.env);
        let ext = ext.unwrap_or_else(|| {
            let mut ext = context.ext.get();
            ext.for_next_transaction(&tx);
            ext
        });
        let factory = self.factory.configure_for(&db, &env);
        // A context that never ran gets its network genesis before the transaction.
        let is_forked = db.is_forked_mode();
        let mut executor = match &context.cache {
            None => factory.create_evm(db, env),
            Some(cache) => {
                let mut executor = factory.new_evm(db, env);
                executor.state_mut().overlay_db_mut().cache = Cache::clone(cache);
                if context.needs_init {
                    factory.initialize_evm(&mut executor, is_forked);
                }
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
        let inactive = Some((id, self.forks.persistent.clone()));
        Ok(PreparedTx { executor, tx, inactive, cursor: None, access_list: None })
    }

    /// Stores the outcome of a [`PreparedTx`]: an inactive context's new accepted state.
    pub fn store_executed(&mut self, executed: ExecutedNested<F::Types>) -> TxResult<F::Types> {
        let ExecutedNested { result, context, .. } = executed;
        if let Some((id, cache)) = context {
            let context = self.forks.context_mut(id).expect("prepared context exists");
            context.cache = Some(Arc::new(cache));
        }
        result
    }

    /// Replaces the accounts the running transaction of `evm` loaded, except persistent ones,
    /// with their accepted state on the inactive fork `id`, or the local context for `None`, as
    /// journaled writes, such as after replaying transactions there.
    ///
    /// Like the REVM backend, which reloaded every account of the running journal from a fork's
    /// database after replaying a transaction on it while another fork was active.
    pub fn rebase_onto_inactive(
        &self,
        evm: &mut Evm<'_, F::Types>,
        id: Option<U256>,
    ) -> eyre::Result<()> {
        eyre::ensure!(id != self.forks.active(), "fork {} is active", id.unwrap_or_default());
        let Some(context) = self.forks.context(id) else {
            eyre::bail!("fork {} does not exist", id.unwrap_or_default());
        };
        let db = context.db.clone_backing();
        let mut source = self.factory.configure_for(&db, &context.env).new_evm(db, context.env);
        if let Some(cache) = &context.cache {
            source.state_mut().overlay_db_mut().cache = Cache::clone(cache);
        }
        rebase_loaded_accounts(evm.state_mut(), source.state_mut(), &self.forks.persistent)?;
        Ok(())
    }

    /// Accepts the running state and stores it, with the running environment, in the active
    /// context. A fork keeps its own chain ID. Returns what the running transaction carries into
    /// the next context.
    fn suspend(&mut self, evm: &mut Evm<'_, F::Types>) -> Suspended {
        // Fold the running transaction's execution so far into its network state before the
        // state is replaced.
        self.factory.sync_transaction(evm);
        let loaded_persistent = evm
            .state()
            .loaded_accounts()
            .map(|(address, _)| *address)
            .filter(|address| self.forks.is_persistent(address))
            .collect::<Vec<_>>();
        for address in loaded_persistent {
            if let Entry::Vacant(entry) = self.tx_original.entry(address) {
                // Read the accepted info without caching it, which would carry it into the next
                // context.
                let db = evm.overlay_db_mut();
                let info = match db.cache.accounts.get(&address) {
                    Some(info) => info.clone(),
                    None => db.db.get_account(&address).ok().flatten(),
                };
                entry.insert(info);
            }
        }
        let state = evm.state_mut();
        let loaded = state
            .loaded_accounts()
            .map(|(address, slots)| (*address, slots.copied().collect::<Vec<_>>()))
            .collect::<Vec<_>>();
        let warm = warm_accounts(state);
        let context = self.active_mut();
        for (address, slots) in loaded {
            if state.account(&address).is_ok_and(|account| account.is_created()) {
                context.created.insert(address);
            }
            // Committing the running transaction below makes the slots' current values their
            // originals, so record the transaction-start ones SSTORE gas and refunds depend on.
            for key in &slots {
                if let Ok(slot) = state.storage_slot(&address, *key)
                    && slot.original() != slot.current()
                {
                    context.originals.entry(address).or_default().insert(*key, slot.original());
                }
            }
            context.loaded.entry(address).or_default().extend(slots);
        }
        // The context keeps the warmth it resumed with, which the running state holds in its
        // prewarm set rather than in loaded accounts, even if this visit did not touch it.
        for (address, slots) in &warm {
            context.warm.entry(*address).or_default().extend(slots);
        }
        let is_view = AcceptedView::is(evm.database());
        let backing = self.active().db.clone_backing();
        let state = evm.state_mut();
        state.commit_transaction();
        let logs = mem::take(state.logs_mut());
        let prewarm = self.tx_prewarm.get_or_insert_with(|| state.prewarm_set().clone()).clone();
        // `resume` replaces the running state, so its committed overlay moves into the context's
        // accepted state and is shared with what the transaction carries over instead of being
        // copied. Dropping the EVM's view first leaves the context the only owner of a shared
        // accepted state, which is then updated in place.
        let overlay = state.overlay_db_mut();
        let local = mem::take(&mut overlay.cache);
        overlay.db = Box::new(backing);
        self.store_env(evm_env(evm));
        let ext = evm.ext().clone();
        let context = self.active_mut();
        let cache = match (is_view, context.cache.take()) {
            (true, Some(mut accepted)) => {
                Arc::make_mut(&mut accepted).merge(local);
                accepted
            }
            _ => Arc::new(local),
        };
        context.cache = Some(Arc::clone(&cache));
        context.switched.clear();
        context.ext = StoredExt::new(&ext);
        Suspended {
            cache,
            loaded: context.loaded.clone(),
            created: context.created.clone(),
            originals: context.originals.clone(),
            logs,
            prewarm,
            warm,
        }
    }

    /// Installs the active context into `evm`, carrying persistent accounts, logs and base warmth
    /// over from the suspended context. With `carry`, the loaded and created accounts of the
    /// suspended context are carried over as well.
    fn resume(
        &mut self,
        evm: &mut Evm<'_, F::Types>,
        suspended: Suspended,
        carry: bool,
        phase: ExecutionPhase,
    ) {
        let Suspended {
            cache: source,
            loaded: source_loaded,
            created: source_created,
            originals: source_originals,
            logs,
            mut prewarm,
            warm: source_warm,
        } = suspended;
        if carry {
            self.active_mut().inherit(&source_loaded, &source_created, &source);
        }
        let context = self.active_mut();
        let fresh = mem::take(&mut context.needs_init) || context.cache.is_none();
        // The running EVM owns the active context's accepted state.
        let mut cache = context.cache.take().map(Arc::unwrap_or_clone).unwrap_or_default();
        let mut loaded = mem::take(&mut context.loaded);
        let db = context.db.clone_backing();
        let env = context.env;
        let ext = context.ext.get();
        self.forks.merge_persistent(&source, &mut cache);
        self.forks.merge_persistent_loaded(&source_loaded, &mut loaded);
        let persistent_created = source_created
            .into_iter()
            .filter(|address| self.forks.is_persistent(address))
            .collect::<Vec<_>>();
        let persistent = self.forks.persistent().clone();
        let context = self.active_mut();
        context.loaded = loaded;
        context.created.retain(|address| {
            !persistent.contains(address) || !source.accounts.contains_key(address)
        });
        context.created.extend(persistent_created);
        let created = context.created.clone();
        // Like REVM's per-fork journals, a selected context resumes with its own warmth from this
        // transaction, and a rolled one warms every account the running transaction loaded, with
        // slots for created and persistent accounts. Persistent accounts keep their warmth.
        let mut warm = mem::take(&mut context.warm);
        if carry {
            for (address, slots) in &source_loaded {
                warm.entry(*address).or_default().extend(slots);
            }
        }
        self.switch_slots.clear();
        for address in &persistent {
            if let Some(slots) = source_warm.get(address) {
                warm.entry(*address).or_default().extend(slots);
                self.switch_slots.insert(*address, slots.clone());
            }
        }
        for (address, slots) in &warm {
            prewarm.warm_storage(address, slots.iter().copied());
        }
        self.active_mut().warm = warm;
        // Persistent and carried accounts keep the originals they suspended with, other accounts
        // the ones of their previous visit to this context. Records this context kept for
        // persistent accounts can predate the running transaction's baseline, such as that of an
        // isolated call, so the suspended context's records replace them.
        let mut originals = mem::take(&mut self.active_mut().originals);
        originals.retain(|address, _| !persistent.contains(address));
        for (address, slots) in source_originals {
            if persistent.contains(&address) || (carry && created.contains(&address)) {
                originals.entry(address).or_default().extend(slots);
            }
        }

        let mut state = State::new(db.clone_backing());
        state.overlay_db_mut().cache = cache;
        *state.logs_mut() = logs;
        *state.prewarm_set_mut() = prewarm;
        *evm.state_mut() = state;
        evm.ext_mut().resume_context(ext);

        let factory = self.factory.configure_for(&db, &env);
        self.apply_env(&factory, evm, env, phase);
        evm.warm_precompiles();
        if fresh {
            factory.initialize_evm(evm, db.is_forked_mode());
        }
        // After the network initialization, which can commit the overlay.
        self.active_mut().switched = restore_originals(evm.state_mut(), &originals);
        // The new transaction overlay does not know which accounts the transaction created, which
        // EIP-6780 `SELFDESTRUCT` depends on. Its database already holds their storage.
        for address in &created {
            if let Ok(mut account) = evm.state_mut().account(address)
                && account.exists()
            {
                account.mark_created_keeping_storage();
            }
        }
        factory.refresh_switched_transaction(evm, &self.tx_original);
    }

    /// Applies `env`, deferring a spec or chain ID change while a transaction is in flight.
    fn apply_env(
        &mut self,
        factory: &F,
        evm: &mut Evm<'_, F::Types>,
        env: EvmEnv<F::Types>,
        phase: ExecutionPhase,
    ) {
        let current = evm_env(evm);
        self.sync_database_policy(evm);
        if current.spec == env.spec && current.chain_id() == env.chain_id() {
            evm.set_block(env.block);
            // `evm` can be a frame EVM that already runs the pending environment while the frames
            // it suspended do not, so the request stays pending for them.
            self.pending_env = match phase {
                ExecutionPhase::InFlight if self.pending_env.is_some() => Some(env),
                _ => None,
            };
            return;
        }
        match phase {
            ExecutionPhase::Boundary => {
                factory.set_env(evm, env);
                self.sync_database_policy(evm);
                self.pending_env = None;
            }
            ExecutionPhase::InFlight => {
                evm.set_block(env.block);
                self.pending_env = Some(env);
            }
        }
    }

    /// Stores `env` as the active context's environment.
    ///
    /// A fork keeps the chain ID it was created or rolled with, as REVM selected a fork with its
    /// own chain ID: a running change such as `vm.chainId` does not follow the fork to its next
    /// selection. The executor carries such a change to its next transaction instead.
    fn store_env(&mut self, mut env: EvmEnv<F::Types>) {
        let is_fork = self.forks.active().is_some();
        let context = self.active_mut();
        if is_fork {
            env.set_chain_id(context.env.chain_id());
        }
        context.env = env;
    }

    /// Returns the warm slots of persistent accounts the last fork switch carried into the next
    /// context.
    pub const fn switch_slots(&self) -> &AddressMap<HashSet<U256>> {
        &self.switch_slots
    }

    /// Returns the accounts loaded in the running transaction, including accounts loaded as
    /// absent and accounts loaded before fork switches, with their loaded slots.
    ///
    /// Accepted state the running transaction did not load is not included.
    pub fn loaded_state(&self, state: &State<'_>) -> AddressMap<HashSet<U256>> {
        let mut loaded = self.active().loaded.clone();
        for (address, slots) in state.loaded_accounts() {
            loaded.entry(*address).or_default().extend(slots);
        }
        loaded
    }

    /// Returns the slots of `address` loaded in the running transaction, including before fork
    /// switches.
    pub fn loaded_storage(&self, state: &State<'_>, address: &Address) -> Vec<U256> {
        let mut slots =
            self.active().loaded.get(address).into_iter().flatten().copied().collect::<Vec<_>>();
        if let Some((_, loaded)) = state.loaded_accounts().find(|(loaded, _)| *loaded == address) {
            slots.extend(loaded);
        }
        slots.sort_unstable();
        slots.dedup();
        slots
    }

    /// Rebinds the running state's backing handle to its EVM's empty-account policy without
    /// copying accepted state or changing another execution's policy.
    pub fn sync_database_policy(&self, evm: &mut Evm<'_, F::Types>) {
        let db = self.forks.active_db();
        db.set_eip161(evm.version().feature(EvmFeatures::EIP161));
        let overlay = evm.state_mut().overlay_db_mut();
        overlay.db = if AcceptedView::is(&*overlay.db) {
            Box::new(AcceptedView {
                cache: Arc::clone(self.active().cache.as_ref().expect("accepted view has a cache")),
                backing: db,
            })
        } else {
            Box::new(db)
        };
    }
}

/// What a running transaction carries from a suspended context into the next one.
struct Suspended {
    /// The suspended context's accepted state, the source of persistent and carried accounts.
    cache: Arc<Cache>,
    /// Accounts and slots loaded in the suspended context.
    loaded: AddressMap<HashSet<U256>>,
    /// Accounts created in the suspended context.
    created: AddressSet,
    /// Transaction-start values of the slots changed in the suspended context.
    originals: AddressMap<HashMap<U256, U256>>,
    /// Logs of the running transaction.
    logs: Vec<Log>,
    /// Base warmth of the running transaction, such as its access list, without warmth carried
    /// by fork switches.
    prewarm: PrewarmSet,
    /// Accounts and slots warm in the running transaction.
    warm: AddressMap<HashSet<U256>>,
}

/// Clears `evm`'s inspector and returns the EVM without the borrows the inspector held, such as
/// to park an EVM whose inspector borrowed the lifecycle's owner.
///
/// # Safety
///
/// An EVM's lifetime bounds its inspector, which is cleared, and its databases and precompiles.
/// Those must outlive `'b`, the lifetime the caller chooses: they must be ones Foundry's
/// factories and [`ExecutionLifecycle`] installed, all of which are owned (`'static`).
pub unsafe fn release_inspector<'b, T: EvmTypes>(mut evm: Evm<'_, T>) -> Evm<'b, T> {
    evm.clear_inspector();
    let evm = mem::ManuallyDrop::new(evm);
    // SAFETY: The inspector is cleared and the caller guarantees the databases and precompiles
    // outlive `'b`. The value is moved out exactly once and the original is never dropped.
    unsafe { std::ptr::read((&raw const *evm).cast::<Evm<'b, T>>()) }
}

/// Applies a nested transaction's changes to the running transaction's `state` as journaled
/// writes and returns them as [`PersistentWrite`]s, for the caller to replay when an enclosing
/// call reverts, such as for `broadcastRawTransaction`.
pub fn merge_untracked_nested_changes<T: EvmTypesHost>(
    state: &mut State<'_>,
    executed: &ExecutedTx<'_, '_, T>,
) -> DbResult<Vec<PersistentWrite>> {
    // As on master, which committed the transaction to the database, accounts and slots the
    // running transaction has not loaded start from its writes.
    let unloaded = unloaded_changes(state, executed.account_changes());
    state.commit_source(&unloaded);
    let mut persistent = Vec::new();
    merge_changes(state, executed.account_changes(), |_| true, Some(&mut persistent), None)?;
    Ok(persistent)
}

/// Applies the changes of a nested transaction that `child` ran and detached into `pending` to the
/// running transaction's `state` like [`merge_untracked_nested_changes`], and also warms the
/// accounts and slots the nested transaction left warm, such as for `executeTransaction`.
///
/// Accounts the running transaction had already loaded keep their warmth, and accesses the nested
/// transaction reverted stay cold. Like the merged changes, the warmth survives reverts of the
/// enclosing calls.
///
/// Existing accounts the nested transaction self-destructed take the state of `destructed`, see
/// [`destructed_accounts`], and are only marked self-destructed. Without one, their balance is
/// emptied and the rest of their state is kept.
pub fn merge_untracked_pending_changes_with_warmth<T: EvmTypes>(
    state: &mut State<'_>,
    child: &mut Evm<'_, T>,
    pending: PendingState,
    mut destructed: AddressMap<DestructedAccount>,
) -> DbResult<Vec<PersistentWrite>> {
    // The detached accounts keep their warmth, which reattaching them to `child` exposes.
    let child = child.state_mut();
    child.set_pending_state(pending.clone());
    let warm = warm_accounts(child);
    child.clear_transaction_state();
    // Warm before merging, so that the merge journals the warmth as the accounts' previous one and
    // reverts of the enclosing calls restore it.
    let loaded = state.loaded_accounts().map(|(address, _)| *address).collect::<AddressSet>();
    for (address, slots) in warm {
        state.account(&address)?;
        if !loaded.contains(&address) {
            state.set_account_warm(&address, true);
        }
        for key in slots {
            state.storage_slot(&address, key)?;
            state.set_storage_warm(&address, key, true);
        }
    }
    // As on master, an existing account the nested transaction self-destructed keeps the state the
    // transaction left it in until the running transaction ends and destroys it.
    let destroyed = pending
        .account_changes()
        .filter(|change| change.selfdestructed && !change.created)
        .map(|change| change.address)
        .collect::<AddressSet>();
    let mut persistent = Vec::new();
    let filter = |address: &Address| !destroyed.contains(address);
    merge_changes(state, pending.account_changes(), filter, Some(&mut persistent), None)?;
    for address in destroyed {
        if let Some(DestructedAccount { info, code, slots }) = destructed.remove(&address) {
            let code = || Ok(code);
            apply_account(state, address, Some(&info), code, false, slots, Some(&mut persistent))?;
        } else {
            let balance = state.account(&address)?.balance();
            let write = PersistentWrite::Balance(address, U256::ZERO.wrapping_sub(balance));
            write.apply(state)?;
            persistent.push(write);
        }
        let write = PersistentWrite::Destruct(address);
        write.apply(state)?;
        persistent.push(write);
    }
    Ok(persistent)
}

/// Applies the changes of a nested transaction detached into `pending` to the running
/// transaction's `state` as journaled writes, so they revert with the enclosing call.
pub fn merge_pending_changes(state: &mut State<'_>, pending: &PendingState) -> DbResult<()> {
    merge_changes(state, pending.account_changes(), |_| true, None, None)
}

/// Returns the warmth of the running transaction in `state`: its base warm set and the accounts and
/// slots it warmed, for a transaction that continues in a replaced state.
pub fn warm_set(state: &mut State<'_>) -> PrewarmSet {
    let mut prewarm = state.prewarm_set().clone();
    for (address, slots) in warm_accounts(state) {
        prewarm.warm_storage(&address, slots);
    }
    prewarm
}

/// Cools every account and slot the running transaction in `state` loaded, without journaling.
/// Pre-warmed ones stay effectively warm.
pub fn cool_loaded(state: &mut State<'_>) {
    let loaded = state
        .loaded_accounts()
        .map(|(address, slots)| (*address, slots.copied().collect::<Vec<_>>()))
        .collect::<Vec<_>>();
    for (address, slots) in loaded {
        state.set_account_warm(&address, false);
        for key in slots {
            state.set_storage_warm(&address, key, false);
        }
    }
}

/// Undoes, without journaling, the warmth that the running transaction in `state` gained since
/// `checkpoint`, keeping all other warmth, including unjournaled changes made since.
pub fn restore_access_warmth(state: &mut State<'_>, checkpoint: &StateCheckpoint) {
    let mut accounts = AddressSet::default();
    let mut slots = Vec::new();
    // A callback that reverted to a snapshot or switched forks can leave a shorter journal.
    for entry in state.journal().get(checkpoint.journal_len()..).unwrap_or_default() {
        match entry {
            // Any account mutation journals an entry; only one taken while cold means it was
            // warmed since.
            JournalEntry::AccountChange { address, previous_flags, .. }
                if !previous_flags.contains(AccountFlags::WARM) =>
            {
                accounts.insert(*address);
            }
            JournalEntry::StorageWarmed { address, key } => slots.push((*address, *key)),
            _ => {}
        }
    }
    for address in accounts {
        state.set_account_warm(&address, false);
    }
    for (address, key) in slots {
        state.set_storage_warm(&address, key, false);
    }
}

/// Returns the accounts the running transaction in `state` loaded and that are warm, with their
/// warm slots.
pub fn warm_accounts(state: &mut State<'_>) -> AddressMap<HashSet<U256>> {
    let loaded = state
        .loaded_accounts()
        .map(|(address, slots)| (*address, slots.copied().collect::<Vec<_>>()))
        .collect::<Vec<_>>();
    let mut warm = AddressMap::default();
    for (address, slots) in loaded {
        if !state.account(&address).is_ok_and(|account| account.is_warm()) {
            continue;
        }
        let slots = slots
            .into_iter()
            .filter(|key| state.storage_slot(&address, *key).is_ok_and(|slot| slot.is_warm()))
            .collect();
        warm.insert(address, slots);
    }
    warm
}

/// Turns the accepted values of the slots of `originals` in the fresh `state` back into pending
/// writes of the running transaction over their transaction-start values.
///
/// The accepted state takes the transaction-start values as well, since committing the
/// transaction only writes slots whose values differ from their originals. Returns the replaced
/// accepted values.
fn restore_originals(
    state: &mut State<'_>,
    originals: &AddressMap<HashMap<U256, U256>>,
) -> AddressMap<HashMap<U256, U256>> {
    let mut pending = PendingState::default();
    let mut switched = AddressMap::<HashMap<_, _>>::default();
    for (address, slots) in originals {
        let Ok(info) = state.account_info_untracked(address) else { continue };
        pending.insert_account(*address, info.clone(), info);
        for (key, original) in slots {
            if let Ok(current) = state.storage_slot_untracked(address, key) {
                pending.insert_storage(*address, *key, *original, current);
                switched.entry(*address).or_default().insert(*key, current);
                let cache = &mut state.overlay_db_mut().cache;
                cache.storage.entry(*address).or_default().slots.insert(*key, *original);
            }
        }
    }
    state.set_pending_state(pending);
    switched
}

/// Applies a nested transaction's `changes` to the accounts selected by `filter` to `state` as
/// journaled writes, also recorded in `persistent` if given.
///
/// Changes only carry changed code, so the code of an account whose code hash differs from
/// `state`'s is read from `source`, the nested transaction's database. Without one, the nested
/// transaction ran over a copy of `state`, where an unchanged code hash is already `state`'s.
fn merge_changes<'c>(
    state: &mut State<'_>,
    changes: impl Iterator<Item = AccountChanges<'c>>,
    filter: impl Fn(&Address) -> bool,
    mut persistent: Option<&mut Vec<PersistentWrite>>,
    mut source: Option<&mut CacheDB<Box<dyn DynDatabase + '_>>>,
) -> DbResult<()> {
    for change in changes.filter(|change| change.is_changed()) {
        let address = change.address;
        if !filter(&address) {
            continue;
        }
        {
            let mut account = state.account(&address)?;
            account.touch();
            // An account the running transaction already created keeps the storage it holds.
            if change.created && !account.is_created() {
                account.mark_created();
            }
        }
        let code = || match (change.code, change.current, source.as_deref_mut()) {
            (Some((_, code)), ..) => Ok(code.clone()),
            (None, Some(info), Some(source)) if !info.is_empty_code_hash() => {
                source.get_code_by_hash(&info.code_hash)
            }
            _ => Ok(Bytecode::default()),
        };
        let wiped = change.current.is_none() || change.storage.wiped;
        let slots = change.storage.changed_slots().map(|(key, slot)| (*key, slot.current));
        apply_account(
            state,
            address,
            change.current,
            code,
            wiped,
            slots,
            persistent.as_deref_mut(),
        )?;
    }
    Ok(())
}

/// Returns the `changes` to accounts and storage slots the running transaction in `state` has not
/// loaded, as a pending state to accept. Accounts that were created, self-destructed, wiped or
/// given code are left out.
fn unloaded_changes<'c>(
    state: &State<'_>,
    changes: impl Iterator<Item = AccountChanges<'c>>,
) -> PendingState {
    let loaded = state
        .loaded_accounts()
        .map(|(address, slots)| (*address, slots.copied().collect::<HashSet<_>>()))
        .collect::<AddressMap<_>>();
    let mut pending = PendingState::default();
    for change in changes.filter(|change| change.is_changed()) {
        if change.created
            || change.selfdestructed
            || change.storage.wiped
            || change.code.is_some()
            || change.current.is_none()
        {
            continue;
        }
        let loaded = loaded.get(&change.address);
        // A loaded account keeps its info, which merges as a journaled write.
        let current = if loaded.is_some() { change.original } else { change.current };
        pending.insert_account(change.address, change.original.cloned(), current.cloned());
        for (key, slot) in change.storage.changed_slots() {
            if !loaded.is_some_and(|slots| slots.contains(key)) {
                pending.insert_storage(change.address, *key, slot.original, slot.current);
            }
        }
    }
    pending
}

/// Replaces the accounts the running transaction in `state` loaded, except persistent ones, with
/// their accepted state in `source`, as journaled writes: info and loaded storage slots.
///
/// Like the REVM backend, which reloaded every account of the running journal from a fork's
/// database after replaying a transaction on that fork while another one was active.
fn rebase_loaded_accounts(
    state: &mut State<'_>,
    source: &mut State<'_>,
    persistent: &AddressSet,
) -> DbResult<()> {
    let loaded = state
        .loaded_accounts()
        .filter(|(address, _)| !persistent.contains(*address))
        .map(|(address, slots)| (*address, slots.copied().collect::<Vec<_>>()))
        .collect::<Vec<_>>();
    for (address, keys) in loaded {
        let info = source.account_info_untracked(&address)?;
        let slots = keys
            .into_iter()
            .map(|key| Ok((key, source.storage_slot_untracked(&address, &key)?)))
            .collect::<DbResult<Vec<_>>>()?;
        let code = || match info.as_ref().map(|info| (info.code_hash, &info.code)) {
            Some((code_hash, None)) if code_hash != KECCAK256_EMPTY => {
                source.overlay_db_mut().get_code_by_hash(&code_hash)
            }
            Some((_, code)) => Ok(code.clone().unwrap_or_default()),
            None => Ok(Bytecode::default()),
        };
        apply_account(state, address, info.as_ref(), code, false, slots, None)?;
    }
    Ok(())
}

/// Sets the account at `address` in `state` to `info`, or to no account, and writes `slots` to
/// its storage after wiping it if `wiped`, as journaled writes, also recorded in `persistent` if
/// given. Fields that already hold their value are left alone; `code` is only built when the code
/// hash changes.
///
/// A journaled wipe is not undone by a revert of the enclosing call: evm2 does not journal the
/// wipe marker, and slots the running transaction never loaded cannot be cleared one by one.
fn apply_account(
    state: &mut State<'_>,
    address: Address,
    info: Option<&AccountInfo>,
    code: impl FnOnce() -> DbResult<Bytecode>,
    wiped: bool,
    slots: impl IntoIterator<Item = (U256, U256)>,
    mut persistent: Option<&mut Vec<PersistentWrite>>,
) -> DbResult<()> {
    let mut record = |write| {
        if let Some(persistent) = persistent.as_deref_mut() {
            persistent.push(write);
        }
    };
    {
        let mut account = state.account(&address)?;
        let (balance, nonce, code_hash) = info.map_or((U256::ZERO, 0, KECCAK256_EMPTY), |info| {
            (info.balance, info.nonce, info.code_hash)
        });
        if account.balance() != balance {
            record(PersistentWrite::Balance(address, balance.wrapping_sub(account.balance())));
            account.set_balance(balance);
        }
        if account.nonce() != nonce {
            let previous = account.nonce();
            account.set_nonce(nonce);
            record(PersistentWrite::nonce(&account, previous));
        }
        if account.code_hash() != code_hash {
            let code = code()?;
            record(PersistentWrite::Code(address, code_hash, code.clone()));
            account.set_code(code_hash, code);
        }
    }
    if wiped {
        record(PersistentWrite::Wipe(address));
        state.storage(&address)?.wipe();
    }
    for (key, value) in slots {
        // A slot the account already holds, such as one the nested transaction only read, is not
        // a write.
        let mut slot = state.storage_slot(&address, key)?;
        if slot.current() != value {
            record(PersistentWrite::Slot(address, key, value));
            slot.set(value);
        }
    }
    Ok(())
}
