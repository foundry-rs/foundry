use alloy_primitives::{Address, B256, StorageKey, StorageValue, U256};
use parking_lot::RwLock;
use reth_ethereum::primitives::Bytecode;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

/// Shared handle to the anvil-owned state writes.
pub type SharedAnvilState = Arc<RwLock<AnvilState>>;

/// A single state write requested through the `anvil_*` namespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateOverride {
    /// Sets the balance of an account.
    Balance(Address, U256),
    /// Sets the nonce of an account.
    Nonce(Address, u64),
    /// Sets the code of an account.
    Code(Address, Bytecode),
    /// Sets one storage slot of an account.
    Storage(Address, StorageKey, StorageValue),
}

/// Anvil-owned state writes.
///
/// A write lands in two places. The read overlay makes it visible at once to RPC reads and to the
/// pool validator. The write also queues for the next block: the block executor applies every
/// queued write at the start of the next built block, so the write enters the block's state
/// changes and state root. Once that block is canonical, the overlay entry is dropped and the
/// chain state serves the value.
#[derive(Clone, Debug, Default)]
pub struct AnvilState {
    accounts: HashMap<Address, AccountOverride>,
    bytecodes: HashMap<B256, Bytecode>,
    /// Writes not yet assigned to a block.
    pending: Vec<StateOverride>,
    /// Writes applied by the block with the given number, awaiting canonicalization.
    frozen: BTreeMap<u64, Vec<StateOverride>>,
    /// Writes applied by canonical blocks, so a replay of a block applies them again.
    applied: BTreeMap<u64, Vec<StateOverride>>,
}

/// The overlay for one account.
#[derive(Debug, Default, Clone)]
pub struct AccountOverride {
    balance: Option<U256>,
    nonce: Option<u64>,
    code_hash: Option<B256>,
    storage: HashMap<StorageKey, StorageValue>,
}

impl AccountOverride {
    /// Returns the overridden balance, if any.
    pub const fn balance(&self) -> Option<U256> {
        self.balance
    }

    /// Returns the overridden nonce, if any.
    pub const fn nonce(&self) -> Option<u64> {
        self.nonce
    }

    /// Returns the overridden code hash, if any.
    pub const fn code_hash(&self) -> Option<B256> {
        self.code_hash
    }

    /// Returns the overridden storage slots.
    pub const fn storage(&self) -> &HashMap<StorageKey, StorageValue> {
        &self.storage
    }
}

impl AnvilState {
    /// Creates a new empty shared state handle.
    pub fn shared() -> SharedAnvilState {
        Arc::new(RwLock::new(Self::default()))
    }

    /// Sets the balance of the given account.
    pub fn set_balance(&mut self, address: Address, balance: U256) {
        self.accounts.entry(address).or_default().balance = Some(balance);
        self.pending.push(StateOverride::Balance(address, balance));
    }

    /// Sets the nonce of the given account.
    pub fn set_nonce(&mut self, address: Address, nonce: u64) {
        self.accounts.entry(address).or_default().nonce = Some(nonce);
        self.pending.push(StateOverride::Nonce(address, nonce));
    }

    /// Sets the code of the given account.
    pub fn set_code(&mut self, address: Address, code: Bytecode) {
        let hash = code.hash_slow();
        self.bytecodes.insert(hash, code.clone());
        self.accounts.entry(address).or_default().code_hash = Some(hash);
        self.pending.push(StateOverride::Code(address, code));
    }

    /// Sets one storage slot of the given account.
    pub fn set_storage_at(&mut self, address: Address, slot: StorageKey, value: StorageValue) {
        self.accounts.entry(address).or_default().storage.insert(slot, value);
        self.pending.push(StateOverride::Storage(address, slot, value));
    }

    /// Returns the overlay for the given account, if any.
    pub fn account(&self, address: &Address) -> Option<&AccountOverride> {
        self.accounts.get(address)
    }

    /// Returns every account overlay.
    pub const fn accounts(&self) -> &HashMap<Address, AccountOverride> {
        &self.accounts
    }

    /// Returns the overridden bytecode for the given code hash, if any.
    pub fn bytecode_by_hash(&self, code_hash: &B256) -> Option<&Bytecode> {
        self.bytecodes.get(code_hash)
    }

    /// Returns the overridden storage value for the given account and slot, if any.
    pub fn storage(&self, address: &Address, slot: &StorageKey) -> Option<StorageValue> {
        self.accounts.get(address).and_then(|account| account.storage.get(slot).copied())
    }

    /// Returns the writes that the block with the given number applies.
    ///
    /// The first call for a block takes every pending write and freezes it for that block, so a
    /// repeated execution of the same block applies the same writes.
    pub fn overrides_for_block(&mut self, number: u64) -> Vec<StateOverride> {
        if let Some(writes) = self.applied.get(&number) {
            return writes.clone();
        }
        // A block whose build failed keeps its writes and takes the ones made since.
        let frozen = self.frozen.entry(number).or_default();
        frozen.append(&mut self.pending);
        frozen.clone()
    }

    /// Drops the overlay entries for writes that the canonical chain now serves, up to and
    /// including the given block number.
    pub fn on_canonical_block(&mut self, number: u64) {
        let applied: Vec<_> = {
            let later = self.frozen.split_off(&(number + 1));
            let applied = std::mem::replace(&mut self.frozen, later);
            self.applied.extend(applied.clone());
            applied.into_values().flatten().collect()
        };
        for write in applied {
            let (address, retained) = match write {
                StateOverride::Balance(address, balance) => (
                    address,
                    self.accounts.get_mut(&address).map(|account| {
                        if account.balance == Some(balance) {
                            account.balance = None;
                        }
                    }),
                ),
                StateOverride::Nonce(address, nonce) => (
                    address,
                    self.accounts.get_mut(&address).map(|account| {
                        if account.nonce == Some(nonce) {
                            account.nonce = None;
                        }
                    }),
                ),
                StateOverride::Code(address, code) => {
                    let hash = code.hash_slow();
                    (
                        address,
                        self.accounts.get_mut(&address).map(|account| {
                            if account.code_hash == Some(hash) {
                                account.code_hash = None;
                            }
                        }),
                    )
                }
                StateOverride::Storage(address, slot, value) => (
                    address,
                    self.accounts.get_mut(&address).map(|account| {
                        if account.storage.get(&slot) == Some(&value) {
                            account.storage.remove(&slot);
                        }
                    }),
                ),
            };
            if retained.is_some()
                && self.accounts.get(&address).is_some_and(|account| {
                    account.balance.is_none()
                        && account.nonce.is_none()
                        && account.code_hash.is_none()
                        && account.storage.is_empty()
                })
            {
                self.accounts.remove(&address);
            }
        }
        let live_hashes: std::collections::HashSet<_> =
            self.accounts.values().filter_map(|account| account.code_hash).collect();
        self.bytecodes.retain(|hash, _| live_hashes.contains(hash));
    }

    /// Queues the writes of the blocks above `number`, which a rewind removed from the chain, for
    /// the next block again. Anvil writes into the state directly, so a rollback or a reorg keeps
    /// them; a revert restores the snapshot's state afterwards and drops them.
    pub fn rewind_to(&mut self, number: u64) {
        let removed: Vec<_> = self
            .applied
            .split_off(&(number + 1))
            .into_values()
            .chain(self.frozen.split_off(&(number + 1)).into_values())
            .flatten()
            .collect();
        let later = std::mem::take(&mut self.pending);
        for write in removed {
            self.apply(write);
        }
        self.pending.extend(later);
    }

    /// Records a write in the overlay and queues it for the next block.
    fn apply(&mut self, write: StateOverride) {
        match write {
            StateOverride::Balance(address, balance) => self.set_balance(address, balance),
            StateOverride::Nonce(address, nonce) => self.set_nonce(address, nonce),
            StateOverride::Code(address, code) => self.set_code(address, code),
            StateOverride::Storage(address, slot, value) => {
                self.set_storage_at(address, slot, value)
            }
        }
    }

    /// Returns the writes the block with the given number applied, or will apply.
    pub fn writes_for_block(&self, number: u64) -> Option<&Vec<StateOverride>> {
        self.frozen.get(&number).or_else(|| self.applied.get(&number))
    }

    /// Builds an overlay that serves the given writes.
    pub fn from_writes(writes: &[StateOverride]) -> Self {
        let mut state = Self::default();
        for write in writes {
            state.apply(write.clone());
        }
        state
    }
}
