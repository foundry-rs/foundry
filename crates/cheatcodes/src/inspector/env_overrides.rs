//! Cheatcode overrides of the `BASEFEE`, `GASPRICE`, `BLOBHASH` and `CHAINID` opcode values.

use alloy_primitives::{B256, U256, map::HashMap};
use foundry_evm_core::backend::LocalForkId;
use std::mem;

/// Env overrides keyed by fork ID (`None` is the local, non-forked state).
type EnvOverridesByFork = HashMap<Option<LocalForkId>, EnvOverrides>;

/// Environment overrides applied at the opcode level.
///
/// evm2 borrows the transaction environment for the whole transaction, and isolated calls run as
/// nested transactions whose fee fields are zeroed for fee accounting, so cheatcodes that change
/// the environment (`vm.fee`, `vm.txGasPrice`, `vm.blobhashes`, `vm.chainId`) cannot rely on the
/// environment for the `BASEFEE`, `GASPRICE`, `BLOBHASH` and `CHAINID` opcodes. These overrides
/// are applied in `step_end` to fix the value that was just pushed onto the stack, and
/// `vm.getBlobhashes()` consults them too.
///
/// Transaction fee accounting and the chain ID used for transaction validation still follow the
/// transaction environment.
#[derive(Clone, Debug, Default)]
pub struct EnvOverrides {
    /// Override for the `BASEFEE` opcode (set via `vm.fee`).
    pub basefee: Option<u64>,
    /// Base fee restored from a snapshot during isolation, valid until the fork is rolled.
    pub implicit_basefee: Option<u64>,
    /// Override for the `GASPRICE` opcode (set via `vm.txGasPrice`).
    pub gas_price: Option<u128>,
    /// Override for the `BLOBHASH` opcode (set via `vm.blobhashes`).
    pub blob_hashes: Option<Vec<B256>>,
    /// Override for the `CHAINID` opcode (set via `vm.chainId`) until the next transaction, which
    /// executes with the new chain ID.
    pub chain_id: Option<u64>,
    /// The opcode about to run (captured in `step`, consumed in `step_end`),
    /// used to know what was just executed when `step_end` fires — at that
    /// point `interpreter.bytecode.opcode()` already points at the *next*
    /// instruction.
    pub(super) pending_opcode: Option<u8>,
    /// Pending index for the `BLOBHASH` opcode, captured in `step` (where
    /// the index is still on top of the stack) for use in `step_end` (after
    /// the opcode has consumed it and pushed the looked-up hash).
    pub(super) pending_blobhash_index: Option<u64>,
}

impl EnvOverrides {
    /// Whether any override is set.
    #[inline]
    pub const fn is_any_set(&self) -> bool {
        self.basefee.is_some()
            || self.implicit_basefee.is_some()
            || self.gas_price.is_some()
            || self.blob_hashes.is_some()
            || self.chain_id.is_some()
    }

    /// Returns the value the `BASEFEE` opcode reads, if overridden.
    #[inline]
    pub fn basefee_override(&self) -> Option<u64> {
        self.basefee.or(self.implicit_basefee)
    }

    /// Returns the value the `GASPRICE` opcode reads, if overridden.
    #[inline]
    pub const fn gas_price_override(&self) -> Option<u128> {
        self.gas_price
    }

    /// Returns the hash the `BLOBHASH` opcode reads at `index`, if overridden.
    ///
    /// Out-of-range indices read the zero hash, per EIP-4844.
    #[inline]
    pub fn blob_hash_override(&self, index: u64) -> Option<B256> {
        let blob_hashes = self.blob_hashes.as_ref()?;
        Some(blob_hashes.get(index as usize).copied().unwrap_or_default())
    }
}

/// Per-fork environment overrides and their state-snapshot copies.
///
/// Snapshot copies are required because backend snapshots do not include inspector state.
#[derive(Clone, Debug, Default)]
pub struct EnvOverrideState {
    by_fork: EnvOverridesByFork,
    snapshots: HashMap<U256, EnvOverridesByFork>,
    /// Whether any fork has an override set, refreshed whenever `by_fork` changes, since the
    /// per-instruction hooks read it.
    any_set: bool,
}

impl EnvOverrideState {
    /// Returns whether no fork has an override entry, active or not.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.by_fork.is_empty()
    }

    /// Returns whether any fork, not only the active one, has an override set.
    #[inline]
    pub const fn is_any_set(&self) -> bool {
        self.any_set
    }

    fn refresh_any_set(&mut self) {
        self.any_set = self.by_fork.values().any(EnvOverrides::is_any_set);
    }

    /// Returns the overrides entry for `fork_id`, even if no override is set.
    #[inline]
    pub fn get(&self, fork_id: Option<LocalForkId>) -> Option<&EnvOverrides> {
        self.by_fork.get(&fork_id)
    }

    /// Returns the overrides entry for `fork_id` mutably, without inserting one.
    #[inline]
    pub(super) fn get_mut(&mut self, fork_id: Option<LocalForkId>) -> Option<&mut EnvOverrides> {
        self.by_fork.get_mut(&fork_id)
    }

    /// Updates the overrides for `fork_id`, inserting an empty entry first if absent.
    pub(crate) fn update(
        &mut self,
        fork_id: Option<LocalForkId>,
        f: impl FnOnce(&mut EnvOverrides),
    ) {
        f(self.by_fork.entry(fork_id).or_default());
        self.refresh_any_set();
    }

    /// Clears the base fee restored for isolation on `fork_id`, without inserting an entry.
    pub(crate) fn clear_implicit_basefee(&mut self, fork_id: Option<LocalForkId>) {
        if let Some(overrides) = self.by_fork.get_mut(&fork_id) {
            overrides.implicit_basefee = None;
        }
        self.refresh_any_set();
    }

    /// Saves all overrides under `snapshot_id`.
    pub(crate) fn save_snapshot(&mut self, snapshot_id: U256) {
        self.snapshots.insert(snapshot_id, self.by_fork.clone());
    }

    /// Restores the overrides saved under `snapshot_id`, deleting the copy if `delete` is set.
    ///
    /// Does nothing if no copy was saved under `snapshot_id`.
    pub(crate) fn restore_snapshot(&mut self, snapshot_id: U256, delete: bool) {
        let snapshot = if delete {
            self.snapshots.remove(&snapshot_id)
        } else {
            self.snapshots.get(&snapshot_id).cloned()
        };
        if let Some(snapshot) = snapshot {
            self.by_fork = snapshot;
            self.refresh_any_set();
        }
    }

    /// Deletes the copy saved under `snapshot_id`.
    pub(crate) fn delete_snapshot(&mut self, snapshot_id: U256) {
        self.snapshots.remove(&snapshot_id);
    }

    /// Deletes all saved copies.
    pub(crate) fn clear_snapshots(&mut self) {
        self.snapshots.clear();
    }

    /// Swaps transaction-specific opcode overrides without changing block overrides or snapshots.
    pub fn swap_transaction_overrides(
        &mut self,
        fork_id: Option<LocalForkId>,
        gas_price: &mut Option<u128>,
        blob_hashes: &mut Option<Vec<B256>>,
    ) {
        let overrides = self.by_fork.entry(fork_id).or_default();
        mem::swap(&mut overrides.gas_price, gas_price);
        mem::swap(&mut overrides.blob_hashes, blob_hashes);
        self.refresh_any_set();
    }

    /// Swaps a fork's current overrides without changing other forks or snapshot copies.
    pub fn swap(&mut self, fork_id: Option<LocalForkId>, overrides: &mut EnvOverrides) {
        mem::swap(self.by_fork.entry(fork_id).or_default(), overrides);
        self.refresh_any_set();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORK: Option<LocalForkId> = Some(U256::from_limbs([1, 0, 0, 0]));
    const SNAPSHOT: U256 = U256::from_limbs([7, 0, 0, 0]);

    #[test]
    fn override_values() {
        let mut overrides = EnvOverrides::default();
        assert_eq!(overrides.basefee_override(), None);
        assert_eq!(overrides.gas_price_override(), None);
        assert_eq!(overrides.blob_hash_override(0), None);

        overrides.implicit_basefee = Some(1);
        assert_eq!(overrides.basefee_override(), Some(1));
        overrides.basefee = Some(2);
        assert_eq!(overrides.basefee_override(), Some(2));

        overrides.gas_price = Some(3);
        assert_eq!(overrides.gas_price_override(), Some(3));

        let hash = B256::repeat_byte(1);
        overrides.blob_hashes = Some(vec![hash]);
        assert_eq!(overrides.blob_hash_override(0), Some(hash));
        assert_eq!(overrides.blob_hash_override(1), Some(B256::ZERO));
        assert_eq!(overrides.blob_hash_override(u64::MAX), Some(B256::ZERO));
    }

    #[test]
    fn tracks_overrides_on_every_fork() {
        let mut state = EnvOverrideState::default();
        assert!(state.is_empty());
        assert!(!state.is_any_set());

        state.update(FORK, |o| o.gas_price = Some(1));
        assert!(!state.is_empty());
        assert!(state.is_any_set());
        assert_eq!(state.get(FORK).and_then(EnvOverrides::gas_price_override), Some(1));

        state.update(None, |o| o.chain_id = Some(3));
        assert!(state.get(None).is_some_and(EnvOverrides::is_any_set), "a chain ID is an override");
        state.update(None, |o| o.chain_id = None);

        state.clear_implicit_basefee(Some(U256::from(2)));
        assert!(state.get(Some(U256::from(2))).is_none(), "clearing must not insert an entry");
        state.update(FORK, |o| o.implicit_basefee = Some(2));
        state.clear_implicit_basefee(FORK);
        assert_eq!(state.get(FORK).and_then(EnvOverrides::basefee_override), None);
    }

    #[test]
    fn restores_snapshots() {
        let mut state = EnvOverrideState::default();
        state.update(None, |o| o.basefee = Some(1));
        state.save_snapshot(SNAPSHOT);

        state.update(None, |o| o.basefee = Some(2));
        state.restore_snapshot(SNAPSHOT, false);
        assert_eq!(state.get(None).and_then(EnvOverrides::basefee_override), Some(1));

        // A kept snapshot can be restored again.
        state.update(None, |o| o.basefee = Some(3));
        state.restore_snapshot(SNAPSHOT, true);
        assert_eq!(state.get(None).and_then(EnvOverrides::basefee_override), Some(1));

        // A deleted snapshot is gone, so restoring it leaves the overrides untouched.
        state.update(None, |o| o.basefee = Some(4));
        state.restore_snapshot(SNAPSHOT, false);
        assert_eq!(state.get(None).and_then(EnvOverrides::basefee_override), Some(4));
    }

    #[test]
    fn deletes_snapshots() {
        let mut state = EnvOverrideState::default();
        state.save_snapshot(SNAPSHOT);
        state.save_snapshot(SNAPSHOT + U256::from(1));
        state.update(None, |o| o.basefee = Some(1));

        state.delete_snapshot(SNAPSHOT);
        state.restore_snapshot(SNAPSHOT, false);
        assert!(state.is_any_set());

        state.clear_snapshots();
        state.restore_snapshot(SNAPSHOT + U256::from(1), false);
        assert!(state.is_any_set());
    }
}
