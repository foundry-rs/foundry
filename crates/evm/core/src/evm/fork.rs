//! Fork registry: which backing databases exist, which one is active, and the accepted state each
//! inactive one resumes with.

use super::{BackingDb, EvmEnv, SendableExt, StoredExt};
use crate::fork::{Fork, ForkId};
use alloy_primitives::{
    Address, U256,
    map::{AddressMap, AddressSet, HashSet},
};
use evm2::{EvmTypesHost, evm::Cache};
use std::{collections::BTreeMap, fmt, sync::Arc};

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
}

impl ForkInfo {
    /// Creates the identity of a fork that is not RPC-backed, such as one over an in-memory
    /// database.
    pub fn new(id: ForkId, url: impl Into<String>, block: Option<u64>) -> Self {
        Self { id, url: url.into(), block, remote: None }
    }
}

/// An execution context that is not running: its backing database, environment, network
/// extension state and accepted state.
pub struct Parked<T: EvmTypesHost> {
    /// Backing database.
    pub db: Box<dyn BackingDb>,
    /// Environment to resume with.
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
        }
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
        if let Some(cache) = &local.cache {
            self.inherit(&local.loaded, &local.created, cache);
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

    /// Returns the accounts that persist across fork switches.
    pub const fn persistent(&self) -> &AddressSet {
        &self.persistent
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
                target.slots.extend(&storage.slots);
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
