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
    /// Slots loaded before suspending this context, cleared at the transaction boundary.
    pub(super) loaded_storage: AddressMap<HashSet<U256>>,
}

impl<T: EvmTypesHost<EvmExt: SendableExt>> Parked<T> {
    /// Creates a context that has never run.
    pub fn new(db: Box<dyn BackingDb>, env: EvmEnv<T>, ext: T::EvmExt) -> Self {
        Self {
            db,
            env,
            ext: StoredExt::new(&ext),
            cache: None,
            loaded_storage: AddressMap::default(),
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
            loaded_storage: self.loaded_storage.clone(),
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

    /// Copies persistent accounts' accepted state from `source` into `target`.
    pub(super) fn merge_persistent(&self, source: &Cache, target: &mut Cache) {
        for address in &self.persistent {
            if let Some(account) = source.accounts.get(address) {
                target.accounts.insert(*address, account.clone());
                if let Some(info) = account
                    && let Some(code) = source.contracts.get(&info.code_hash)
                {
                    target.contracts.insert(info.code_hash, code.clone());
                }
            }
            match source.storage.get(address) {
                Some(storage) => {
                    target.storage.insert(*address, storage.clone());
                }
                None => {
                    target.storage.remove(address);
                }
            }
        }
    }

    /// Copies persistent accounts' transaction-loaded slots alongside their accepted state.
    pub(super) fn merge_persistent_loaded_storage(
        &self,
        source: &AddressMap<HashSet<U256>>,
        target: &mut AddressMap<HashSet<U256>>,
    ) {
        for address in &self.persistent {
            match source.get(address) {
                Some(slots) => {
                    target.insert(*address, slots.clone());
                }
                None => {
                    target.remove(address);
                }
            }
        }
    }
}
