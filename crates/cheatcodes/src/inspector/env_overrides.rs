//! Cheatcode overrides of the `BASEFEE`, `GASPRICE` and `BLOBHASH` opcode values.

use alloy_primitives::{B256, U256, map::HashMap};
use foundry_evm_core::backend::LocalForkId;

/// Env overrides keyed by fork ID (`None` is the local, non-forked state).
type EnvOverridesByFork = HashMap<Option<LocalForkId>, EnvOverrides>;

/// Environment overrides applied at the opcode level.
///
/// In isolation mode (and inside the synthetic transactions used by
/// `--gas-report` / `--isolate`) the transaction environment is zeroed for
/// fee-accounting purposes, so cheatcodes that mutate the env (e.g.
/// `vm.fee`, `vm.txGasPrice`, `vm.blobhashes`) cannot rely on those
/// mutations being visible to contracts via the `BASEFEE`, `GASPRICE` and
/// `BLOBHASH` opcodes. These overrides are applied in `step_end` to fix
/// the value that was just pushed onto the stack.
///
/// # Semantics when invoked from inside the synthetic isolation transaction
///
/// `vm.fee` / `vm.txGasPrice` / `vm.blobhashes` consult
/// [`Cheatcodes::in_isolation_context`](crate::Cheatcodes::in_isolation_context); when set, they
/// only update these overrides (so `tx.gas_price = 0` continues to apply to fee accounting and
/// EIP-4844 inner-tx validation does not reject the synthetic call) and
/// leave the real env untouched. After the inner transaction returns, the
/// outer env is restored from the cached snapshot taken before
/// `transact_inner`, which means:
///
/// - the override **does** persist for subsequent `BASEFEE`, `GASPRICE` and `BLOBHASH` reads (this
///   hook fires in `step_end` regardless of isolation),
/// - `vm.getBlobhashes()` also consults these overrides, so it returns the correct value.
/// - but the real `block.basefee` / `tx.gas_price` / `tx.blob_hashes` do **not** reflect the
///   cheatcode value, so other non-opcode env consumers will not see it.
///
/// Calling these cheatcodes outside isolation behaves as before (real env
/// is also mutated and the override mirrors it).
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
    /// `tx.gas_price` captured at snapshot time when no gas_price override was
    /// active. `sync_tx_after_env_override_restore` uses this to restore the
    /// real pre-override value (not hardcoded 0) on revert.
    pub pre_override_gas_price: Option<u128>,
    /// `tx.tx_type` captured at snapshot time when no blob_hashes override was
    /// active. Prevents tx_type being stuck at EIP4844 after reverting from a
    /// blobhashes-set state.
    pub pre_override_tx_type: Option<u8>,
    /// `tx.blob_hashes` captured at snapshot time when no blob_hashes override
    /// was active.
    pub pre_override_blob_hashes: Option<Vec<B256>>,
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
}

impl EnvOverrideState {
    /// Returns whether no fork has an override entry, active or not.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.by_fork.is_empty()
    }

    /// Returns whether any fork, not only the active one, has an override set.
    #[inline]
    pub fn is_any_set(&self) -> bool {
        self.by_fork.values().any(EnvOverrides::is_any_set)
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
    }

    /// Clears the base fee restored for isolation on `fork_id`, without inserting an entry.
    pub(crate) fn clear_implicit_basefee(&mut self, fork_id: Option<LocalForkId>) {
        if let Some(overrides) = self.by_fork.get_mut(&fork_id) {
            overrides.implicit_basefee = None;
        }
    }

    /// Removes the entry for `fork_id` if no override is set on it.
    pub(crate) fn remove_if_unset(&mut self, fork_id: Option<LocalForkId>) {
        if self.by_fork.get(&fork_id).is_some_and(|overrides| !overrides.is_any_set()) {
            self.by_fork.remove(&fork_id);
        }
    }

    /// Saves all overrides, recording the active fork's non-overridden transaction values.
    pub(crate) fn save_snapshot(
        &mut self,
        snapshot_id: U256,
        active_fork_id: Option<LocalForkId>,
        tx_gas_price: u128,
        tx_type: u8,
        tx_blob_hashes: &[B256],
    ) {
        let mut snapshot = self.by_fork.clone();
        let active = snapshot.entry(active_fork_id).or_default();
        if active.gas_price.is_none() {
            active.pre_override_gas_price = Some(tx_gas_price);
        }
        if active.blob_hashes.is_none() {
            active.pre_override_tx_type = Some(tx_type);
            active.pre_override_blob_hashes = Some(tx_blob_hashes.to_vec());
        }
        self.snapshots.insert(snapshot_id, snapshot);
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

        state.update(None, |_| {});
        assert!(!state.is_empty());
        assert!(!state.is_any_set());

        state.update(FORK, |o| o.gas_price = Some(1));
        assert!(state.is_any_set());

        state.remove_if_unset(None);
        state.remove_if_unset(FORK);
        assert!(state.get(None).is_none());
        assert_eq!(state.get(FORK).and_then(EnvOverrides::gas_price_override), Some(1));

        state.clear_implicit_basefee(None);
        assert!(state.get(None).is_none(), "clearing must not insert an entry");
        state.update(FORK, |o| o.implicit_basefee = Some(2));
        state.clear_implicit_basefee(FORK);
        assert_eq!(state.get(FORK).and_then(EnvOverrides::basefee_override), None);
    }

    #[test]
    fn save_snapshot_records_tx_values_that_are_not_overridden() {
        let mut state = EnvOverrideState::default();
        let tx_hashes = [B256::repeat_byte(1)];

        state.save_snapshot(SNAPSHOT, None, 10, 3, &tx_hashes);
        let snapshot = &state.snapshots[&SNAPSHOT];
        let active = &snapshot[&None];
        assert!(!active.is_any_set());
        assert_eq!(active.pre_override_gas_price, Some(10));
        assert_eq!(active.pre_override_tx_type, Some(3));
        assert_eq!(active.pre_override_blob_hashes.as_deref(), Some(&tx_hashes[..]));
        assert!(state.is_empty(), "saving a snapshot must not change the live overrides");

        state.update(None, |o| {
            o.gas_price = Some(1);
            o.blob_hashes = Some(vec![]);
        });
        state.update(FORK, |o| o.basefee = Some(2));
        state.save_snapshot(SNAPSHOT, None, 10, 3, &tx_hashes);
        let snapshot = &state.snapshots[&SNAPSHOT];
        let active = &snapshot[&None];
        assert_eq!(active.pre_override_gas_price, None);
        assert_eq!(active.pre_override_tx_type, None);
        assert_eq!(active.pre_override_blob_hashes, None);
        assert_eq!(snapshot[&FORK].pre_override_gas_price, None, "only the active fork");
    }

    #[test]
    fn restores_snapshots() {
        let mut state = EnvOverrideState::default();
        state.update(None, |o| o.basefee = Some(1));
        state.save_snapshot(SNAPSHOT, None, 0, 0, &[]);

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
        state.save_snapshot(SNAPSHOT, None, 0, 0, &[]);
        state.save_snapshot(SNAPSHOT + U256::from(1), None, 0, 0, &[]);
        state.update(None, |o| o.basefee = Some(1));

        state.delete_snapshot(SNAPSHOT);
        state.restore_snapshot(SNAPSHOT, false);
        assert!(state.is_any_set());

        state.clear_snapshots();
        state.restore_snapshot(SNAPSHOT + U256::from(1), false);
        assert!(state.is_any_set());
    }
}
