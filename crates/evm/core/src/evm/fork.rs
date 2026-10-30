//! Fork registry: which backing databases exist, which one is active, and the accepted state each
//! inactive one resumes with.

use super::{AcceptedView, BackingDb, EvmEnv, SendableExt, StoredExt};
use crate::fork::{Fork, ForkId};
use alloy_eips::BlockNumHash;
use alloy_primitives::{
    Address, B256, U256,
    map::{AddressMap, AddressSet, HashMap, HashSet},
};
use evm2::{EvmFeatures, EvmTypesHost, evm::Cache};
use std::{collections::BTreeMap, fmt, mem, sync::Arc};

/// Identity of a fork as Foundry users see it: its RPC endpoint and pinned block.
///
/// It travels with the fork's context, so snapshot reverts restore it together with the state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkInfo {
    /// Deduplicating identifier of the RPC endpoint and block.
    pub id: ForkId,
    /// RPC endpoint.
    pub url: String,
    /// RPC block number whose state the fork executes on, if any.
    ///
    /// After rolling to a transaction this is the transaction's block, whose parent state was
    /// replayed up to the transaction.
    pub block: Option<u64>,
    /// The remote fork backing the context, if it is RPC-backed.
    pub remote: Option<Fork>,
    /// Canonical chain position of the fork's accepted state, for networks whose execution
    /// depends on it, such as Monad.
    ///
    /// The matching block cursor is retained in the context's network extension state; both move
    /// together on canonical replays and are restored together by snapshot reverts. Synthetic
    /// transactions never advance it.
    pub position: Option<ForkPosition>,
}

impl ForkInfo {
    /// Creates the identity of a fork that is not RPC-backed, such as one over an in-memory
    /// database.
    pub fn new(id: ForkId, url: impl Into<String>, block: Option<u64>) -> Self {
        Self { id, url: url.into(), block, remote: None, position: None }
    }
}

/// Canonical chain position of a fork's accepted state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForkPosition {
    /// The state contains every transaction through `block`.
    AfterBlock(BlockNumHash),
    /// The state contains the transactions of `block` before `index`.
    BeforeTransaction {
        /// Block whose transactions are partially applied.
        block: BlockNumHash,
        /// Index of the next canonical transaction in `block`.
        index: usize,
    },
}

impl ForkPosition {
    /// Returns the position after the transaction at `index` of `block`, whose parent is
    /// `parent_hash` and which has `count` transactions, if that transaction immediately follows
    /// this position.
    pub fn after_transaction(
        self,
        block: BlockNumHash,
        parent_hash: B256,
        index: usize,
        count: usize,
    ) -> Option<Self> {
        let is_next = match self {
            Self::AfterBlock(previous) => {
                index == 0
                    && previous.number.checked_add(1) == Some(block.number)
                    && previous.hash == parent_hash
            }
            Self::BeforeTransaction { block: current, index: next } => {
                current == block && next == index
            }
        };
        let next = index.checked_add(1)?;
        if !is_next || next > count {
            return None;
        }
        Some(if next == count {
            Self::AfterBlock(block)
        } else {
            Self::BeforeTransaction { block, index: next }
        })
    }
}

/// An execution context that is not running: its backing database, environment, network
/// extension state and accepted state.
pub struct Parked<T: EvmTypesHost> {
    /// Backing database.
    pub db: Box<dyn BackingDb>,
    /// Environment to resume with. A fork keeps the chain ID it was created or rolled with,
    /// whatever chain ID it last ran with.
    pub env: EvmEnv<T>,
    /// Network extension state to resume with.
    pub ext: StoredExt<T::EvmExt>,
    /// Accepted state to resume with; `None` until the context is first initialized.
    ///
    /// The cache is shared with snapshots and with EVMs created over the context, so taking a
    /// snapshot or starting an execution does not copy it.
    pub cache: Option<Arc<Cache>>,
    /// Whether `cache` holds carried accounts but the network initialization of a context that
    /// never ran is still due.
    pub(super) needs_init: bool,
    /// Accounts loaded in this context before it was suspended, with their loaded slots. Like
    /// REVM's per-fork journals, they are cleared when the context finishes a transaction.
    pub(super) loaded: AddressMap<HashSet<U256>>,
    /// Accounts created in this context before it was suspended, with the same lifetime as
    /// `loaded`.
    pub(super) created: AddressSet,
    /// Accounts warm in this context when the running transaction last suspended it, with their
    /// warm slots. Cleared between transactions, as REVM treats journal entries of earlier
    /// transactions as cold.
    pub(super) warm: AddressMap<HashSet<U256>>,
    /// Transaction-start values of the slots the running transaction changed in this context
    /// before it last suspended it, which its accepted state no longer holds. Cleared between
    /// transactions, like `warm`.
    pub(super) originals: AddressMap<HashMap<U256, U256>>,
    /// Values at the last fork switch of the slots whose transaction-start originals the running
    /// transaction restored on resuming this context, which a revert that leaves the context
    /// keeps as its accepted state. Cleared when the context is suspended and between
    /// transactions.
    pub(super) switched: AddressMap<HashMap<U256, U256>>,
}

impl<T: EvmTypesHost<EvmExt: SendableExt>> Parked<T> {
    /// Creates a context that has never run.
    pub fn new(db: Box<dyn BackingDb>, env: EvmEnv<T>, ext: T::EvmExt) -> Self {
        Self {
            db,
            env,
            ext: StoredExt::new(&ext),
            cache: None,
            needs_init: false,
            loaded: AddressMap::default(),
            created: AddressSet::default(),
            warm: AddressMap::default(),
            originals: AddressMap::default(),
            switched: AddressMap::default(),
        }
    }

    /// Returns a database reading this context's accepted state over its backing database, with
    /// the context's empty-account policy, for reads between executions.
    pub fn accepted_db(&self) -> impl BackingDb + use<T> {
        let backing = self.db.clone_backing();
        backing.set_eip161(self.env.version.feature(EvmFeatures::EIP161));
        AcceptedView { cache: self.cache.clone().unwrap_or_default(), backing }
    }

    /// Makes the `loaded` accounts and slots visible in this context and carries the `created`
    /// accounts over from `source` with wiped storage.
    ///
    /// Other accounts keep this context's state, so their values are never stale copies from
    /// another fork.
    pub(super) fn inherit(
        &mut self,
        loaded: &AddressMap<HashSet<U256>>,
        created: &AddressSet,
        source: &Cache,
    ) {
        for (address, slots) in loaded {
            self.loaded.entry(*address).or_default().extend(slots);
        }
        if created.is_empty() {
            return;
        }
        self.needs_init |= self.cache.is_none();
        let target = Arc::make_mut(self.cache.get_or_insert_default());
        for address in created {
            copy_account(address, source, target);
            let mut storage = source.storage.get(address).cloned().unwrap_or_default();
            storage.wiped = true;
            target.storage.insert(*address, storage);
        }
        self.created.extend(created);
    }

    /// Starts this fork from what the running transaction loaded and created in the `local`
    /// context before it first selected a fork. Like REVM's initial journal, loaded storage values
    /// are kept even when a non-created account's info comes from the fork.
    pub(super) fn inherit_local(&mut self, local: &Self) {
        for (address, slots) in &local.warm {
            self.warm.entry(*address).or_default().extend(slots);
        }
        if let Some(cache) = &local.cache {
            self.inherit(&local.loaded, &local.created, cache);
            for (address, slots) in &local.originals {
                self.originals.entry(*address).or_default().extend(slots);
            }
            for (address, slots) in &local.loaded {
                if slots.is_empty() {
                    continue;
                }
                self.needs_init |= self.cache.is_none();
                let target = Arc::make_mut(self.cache.get_or_insert_default());
                let target = target.storage.entry(*address).or_default();
                let source = cache.storage.get(address);
                for slot in slots {
                    let value = source
                        .and_then(|storage| storage.slots.get(slot))
                        .copied()
                        .unwrap_or_default();
                    target.slots.insert(*slot, value);
                }
            }
        }
    }

    /// Returns the accounts and slots the running transaction is warm in this context from its
    /// earlier visits, which fork switches carry back in when it resumes.
    pub const fn carried_warmth(&self) -> &AddressMap<HashSet<U256>> {
        &self.warm
    }
}

impl<T: EvmTypesHost<EvmExt: SendableExt>> Clone for Parked<T> {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone_backing(),
            env: self.env,
            ext: self.ext.clone(),
            cache: self.cache.clone(),
            needs_init: self.needs_init,
            loaded: self.loaded.clone(),
            created: self.created.clone(),
            warm: self.warm.clone(),
            originals: self.originals.clone(),
            switched: self.switched.clone(),
        }
    }
}

impl<T: EvmTypesHost> fmt::Debug for Parked<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Parked").field("cache", &self.cache.is_some()).finish_non_exhaustive()
    }
}

/// A registered fork.
pub struct ForkEntry<T: EvmTypesHost> {
    /// User-visible identity.
    pub info: ForkInfo,
    /// The fork's context. Its accepted state is stale while the fork is active.
    pub context: Parked<T>,
}

impl<T: EvmTypesHost> fmt::Debug for ForkEntry<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForkEntry").field("info", &self.info).finish_non_exhaustive()
    }
}

impl<T: EvmTypesHost<EvmExt: SendableExt>> Clone for ForkEntry<T> {
    fn clone(&self) -> Self {
        Self { info: self.info.clone(), context: self.context.clone() }
    }
}

/// Registered forks, the local context, the active selection and persistent accounts.
///
/// The active context's accepted state lives in the running EVM; every other context keeps its
/// own here. Switching is driven by [`ExecutionLifecycle`](super::ExecutionLifecycle).
pub struct ForkRegistry<T: EvmTypesHost> {
    /// The non-forked context.
    pub(super) local: Parked<T>,
    /// Forks by local fork ID.
    pub(super) forks: BTreeMap<U256, ForkEntry<T>>,
    /// Active fork, or `None` for the local context.
    pub(super) active: Option<U256>,
    /// Next local fork ID. Never reused, even across snapshot reverts.
    pub(super) next_id: U256,
    /// Accounts whose state follows every fork switch.
    pub(super) persistent: AddressSet,
}

impl<T: EvmTypesHost> fmt::Debug for ForkRegistry<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForkRegistry")
            .field("forks", &self.forks)
            .field("active", &self.active)
            .field("persistent", &self.persistent)
            .finish_non_exhaustive()
    }
}

impl<T: EvmTypesHost<EvmExt: SendableExt>> Clone for ForkRegistry<T> {
    fn clone(&self) -> Self {
        Self {
            local: self.local.clone(),
            forks: self.forks.clone(),
            active: self.active,
            next_id: self.next_id,
            persistent: self.persistent.clone(),
        }
    }
}

impl<T: EvmTypesHost> ForkRegistry<T> {
    /// Creates a registry whose active context is the local one.
    pub fn new(local: Parked<T>) -> Self {
        Self {
            local,
            forks: BTreeMap::new(),
            active: None,
            next_id: U256::ZERO,
            persistent: AddressSet::default(),
        }
    }

    /// Registers a fork and returns its local fork ID. The fork is not selected.
    pub fn insert(&mut self, info: ForkInfo, context: Parked<T>) -> U256 {
        let id = self.next_id;
        self.next_id += U256::ONE;
        self.forks.insert(id, ForkEntry { info, context });
        id
    }

    /// Returns the active fork, or `None` for the local context.
    pub const fn active(&self) -> Option<U256> {
        self.active
    }

    /// Returns a registered fork.
    pub fn get(&self, id: U256) -> Option<&ForkEntry<T>> {
        self.forks.get(&id)
    }

    /// Returns a registered fork mutably.
    pub fn get_mut(&mut self, id: U256) -> Option<&mut ForkEntry<T>> {
        self.forks.get_mut(&id)
    }

    /// Returns the number of registered forks.
    pub fn len(&self) -> usize {
        self.forks.len()
    }

    /// Returns whether no fork is registered.
    pub fn is_empty(&self) -> bool {
        self.forks.is_empty()
    }

    /// Returns the registered forks.
    pub fn iter(&self) -> impl Iterator<Item = (U256, &ForkEntry<T>)> {
        self.forks.iter().map(|(id, entry)| (*id, entry))
    }

    /// Returns a context, or `None` for an unknown fork.
    ///
    /// The active context's accepted state is stale while an EVM runs it.
    pub fn context(&self, id: Option<U256>) -> Option<&Parked<T>> {
        match id {
            None => Some(&self.local),
            Some(id) => self.forks.get(&id).map(|entry| &entry.context),
        }
    }

    /// Returns a context mutably, or `None` for an unknown fork.
    pub fn context_mut(&mut self, id: Option<U256>) -> Option<&mut Parked<T>> {
        match id {
            None => Some(&mut self.local),
            Some(id) => self.forks.get_mut(&id).map(|entry| &mut entry.context),
        }
    }

    /// Returns a new handle to the active context's backing database.
    pub fn active_db(&self) -> Box<dyn BackingDb> {
        self.context(self.active).expect("active context exists").db.clone_backing()
    }

    /// Marks an account as persistent across fork switches. Returns whether it was not yet
    /// persistent.
    pub fn add_persistent(&mut self, address: Address) -> bool {
        self.persistent.insert(address)
    }

    /// Stops an account from persisting across fork switches. Returns whether it was persistent.
    pub fn remove_persistent(&mut self, address: &Address) -> bool {
        self.persistent.remove(address)
    }

    /// Returns whether an account persists across fork switches.
    pub fn is_persistent(&self, address: &Address) -> bool {
        self.persistent.contains(address)
    }

    /// Merges persistent accounts' accepted state from `source` into `target`.
    ///
    /// Slots of `source` override those of `target`, and other `target` slots are kept unless
    /// `source` wiped the account's storage.
    pub(super) fn merge_persistent(&self, source: &Cache, target: &mut Cache) {
        for address in &self.persistent {
            copy_account(address, source, target);
            if let Some(storage) = source.storage.get(address) {
                let target = target.storage.entry(*address).or_default();
                if storage.wiped {
                    target.wipe();
                }
                // Copying the source's slot table and looking up the target's own slots is
                // cheaper than reinserting every source slot, as the source usually holds them all.
                let kept = mem::replace(&mut target.slots, storage.slots.clone());
                for (key, value) in kept {
                    target.slots.entry(key).or_insert(value);
                }
            }
        }
    }

    /// Makes persistent accounts loaded in `source` visible in `target`, along with their slots.
    pub(super) fn merge_persistent_loaded(
        &self,
        source: &AddressMap<HashSet<U256>>,
        target: &mut AddressMap<HashSet<U256>>,
    ) {
        for address in &self.persistent {
            if let Some(slots) = source.get(address) {
                target.entry(*address).or_default().extend(slots);
            }
        }
    }
}

/// Copies the account info and code of `address` from `source` into `target`, if `source` holds
/// the account.
fn copy_account(address: &Address, source: &Cache, target: &mut Cache) {
    if let Some(account) = source.accounts.get(address) {
        target.accounts.insert(*address, account.clone());
        if let Some(info) = account
            && let Some(code) = source.contracts.get(&info.code_hash)
        {
            target.contracts.insert(info.code_hash, code.clone());
        }
    }
}
