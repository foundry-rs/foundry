//! Snapshot fuzzing: resume invariant runs from interesting states reached by earlier runs.
//!
//! After every accepted call the storage writes are compressed into features. A slot's values are
//! a set, each value its own feature, until the slot took more than MAX_SET_VALUES distinct
//! values; from then on they are a distribution, binned by sign-folded log2. Mapping and array
//! entries are folded per contract, plus the log2 of how many of them are nonzero.
//!
//! A call that reaches a feature no earlier call reached makes its post-call state a snapshot: the
//! executor is cloned together with the call prefix that produced it. Later runs start either from
//! the post-setUp state or from a snapshot, picked by a power schedule that favors snapshots whose
//! features are still rare and whose runs keep finding new coverage or new states. Runs started
//! from a snapshot carry its prefix in their inputs, so counterexamples, shrinking, replay and
//! corpus entries still start from the post-setUp state.

use crate::executors::Executor;
use alloy_primitives::{
    Address, U256,
    map::{AddressMap, DefaultHashBuilder, HashMap, HashSet},
};
use foundry_evm_core::{constants::CHEATCODE_ADDRESS, evm::FoundryEvmNetwork};
use foundry_evm_fuzz::BasicTxDetails;
use proptest::{prelude::Rng, test_runner::TestRunner};
use revm::state::Account;
use std::{hash::BuildHasher, sync::Arc};

/// Most snapshots kept per worker.
const MAX_SNAPSHOTS: usize = 64;
/// Snapshots are only taken while the reproducing sequence is at most this many run depths long.
const MAX_PREFIX_DEPTHS: usize = 4;
/// Distinct values a slot may take before it is treated as a distribution rather than a set.
const MAX_SET_VALUES: usize = 64;

/// Slots at or above 2^128 are treated as computed (mapping or dynamic array) slots.
const COMPUTED_SLOT: U256 = U256::from_limbs([0, 0, 1, 0]);

/// Sign-folded bit length: small positive and small negative (two's complement) values land in
/// different, magnitude-ordered bins.
fn log2_bin(value: U256) -> u16 {
    if value.bit(255) { 512 - (!value).bit_len() as u16 } else { value.bit_len() as u16 }
}

#[derive(Clone, Copy, Default)]
struct Energy {
    picks: u32,
    finds: u32,
}

impl Energy {
    fn productivity(self) -> f64 {
        f64::from(self.finds + 1) / f64::from(self.picks + 1)
    }
}

struct Snapshot<FEN: FoundryEvmNetwork> {
    executor: Executor<FEN>,
    prefix: Arc<[BasicTxDetails]>,
    /// Nonzero computed slots per contract in this state.
    cardinality: AddressMap<u64>,
    /// Features first reached by the call that produced this state.
    features: Vec<u64>,
    energy: Energy,
}

/// Where the current run started.
#[derive(Clone, Copy)]
enum Start {
    Setup,
    Snapshot(usize),
}

/// The state a run starts from.
pub(super) struct RunStart<FEN: FoundryEvmNetwork> {
    pub executor: Executor<FEN>,
    pub prefix: Arc<[BasicTxDetails]>,
}

/// Per-worker pool of snapshots and the state features seen so far.
pub(super) struct StateSnapshots<FEN: FoundryEvmNetwork> {
    max_prefix: usize,
    hasher: DefaultHashBuilder,
    /// Feature -> number of calls that reached it.
    hits: HashMap<u64, u32>,
    /// Distinct values per (contract, slot) until the slot becomes a distribution.
    slot_values: HashMap<(Address, U256), HashSet<U256>>,
    setup: Energy,
    snapshots: Vec<Snapshot<FEN>>,
    current: Start,
    /// Nonzero computed slots per contract in the current run.
    cardinality: AddressMap<u64>,
    /// Whether the current run reached a new feature.
    run_found: bool,
}

impl<FEN: FoundryEvmNetwork> StateSnapshots<FEN> {
    pub(super) fn new(depth: u32) -> Self {
        Self {
            max_prefix: (depth as usize).saturating_mul(MAX_PREFIX_DEPTHS),
            hasher: DefaultHashBuilder::default(),
            hits: HashMap::default(),
            slot_values: HashMap::default(),
            setup: Energy::default(),
            snapshots: Vec::new(),
            current: Start::Setup,
            cardinality: AddressMap::default(),
            run_found: false,
        }
    }

    fn rarity(&self, snapshot: &Snapshot<FEN>) -> f64 {
        snapshot
            .features
            .iter()
            .map(|feature| 1.0 / f64::from(self.hits.get(feature).copied().unwrap_or(1).max(1)))
            .sum()
    }

    fn weight(&self, snapshot: &Snapshot<FEN>) -> f64 {
        self.rarity(snapshot) * snapshot.energy.productivity()
    }

    /// Picks the state the next run starts from.
    pub(super) fn start(
        &mut self,
        runner: &mut TestRunner,
        setup: &Executor<FEN>,
    ) -> RunStart<FEN> {
        self.run_found = false;
        self.current = self.pick(runner);
        match self.current {
            Start::Setup => {
                self.setup.picks += 1;
                self.cardinality.clear();
                RunStart { executor: setup.clone(), prefix: Arc::from([]) }
            }
            Start::Snapshot(index) => {
                let snapshot = &mut self.snapshots[index];
                snapshot.energy.picks += 1;
                self.cardinality.clone_from(&snapshot.cardinality);
                RunStart { executor: snapshot.executor.clone(), prefix: snapshot.prefix.clone() }
            }
        }
    }

    fn pick(&self, runner: &mut TestRunner) -> Start {
        if self.snapshots.is_empty() {
            return Start::Setup;
        }
        // The setup state counts as one fresh feature, so it keeps its share as snapshot
        // features become common.
        let setup = self.setup.productivity();
        let weights = self.snapshots.iter().map(|s| self.weight(s)).collect::<Vec<_>>();
        let total = setup + weights.iter().sum::<f64>();
        let mut point = runner.rng().random::<f64>() * total;
        if point < setup {
            return Start::Setup;
        }
        point -= setup;
        for (index, weight) in weights.iter().enumerate() {
            if point < *weight {
                return Start::Snapshot(index);
            }
            point -= weight;
        }
        Start::Snapshot(self.snapshots.len() - 1)
    }

    fn feature(&self, parts: impl std::hash::Hash) -> u64 {
        self.hasher.hash_one(parts)
    }

    /// Compresses an accepted call's storage writes into features and returns the ones no call
    /// reached before.
    pub(super) fn observe(&mut self, changeset: &AddressMap<Account>) -> Vec<u64> {
        let mut features = Vec::new();
        for (address, account) in changeset {
            if *address == CHEATCODE_ADDRESS {
                continue;
            }
            let mut cardinality_changed = false;
            for (slot, value) in &account.storage {
                if !value.is_changed() {
                    continue;
                }
                let present = value.present_value;
                let computed = *slot >= COMPUTED_SLOT;
                // Mapping keys and array indices are not part of the state's shape: fold them.
                let slot_key = if computed { U256::MAX } else { *slot };
                if computed && value.original_value.is_zero() != present.is_zero() {
                    let count = self.cardinality.entry(*address).or_default();
                    if present.is_zero() {
                        *count = count.saturating_sub(1);
                    } else {
                        *count += 1;
                    }
                    cardinality_changed = true;
                }
                let values = self.slot_values.entry((*address, slot_key)).or_default();
                if values.len() <= MAX_SET_VALUES {
                    values.insert(present);
                }
                let bin = if values.len() <= MAX_SET_VALUES {
                    (0u8, present)
                } else {
                    (1, U256::from(log2_bin(present)))
                };
                features.push(self.feature((address, slot_key, bin)));
            }
            if cardinality_changed {
                let count = self.cardinality.get(address).copied().unwrap_or_default();
                features.push(self.feature((address, 2u8, 64 - count.leading_zeros())));
            }
        }
        let mut novel = Vec::new();
        for feature in features {
            let hits = self.hits.entry(feature).or_default();
            if *hits == 0 {
                novel.push(feature);
            }
            *hits = hits.saturating_add(1);
        }
        novel.sort_unstable();
        novel.dedup();
        if !novel.is_empty() {
            self.run_found = true;
        }
        novel
    }

    /// Keeps the executor's state as a snapshot reached by inputs, which reached novel features.
    pub(super) fn capture(
        &mut self,
        executor: &Executor<FEN>,
        inputs: &[BasicTxDetails],
        novel: Vec<u64>,
    ) {
        if novel.is_empty() || inputs.is_empty() || inputs.len() > self.max_prefix {
            return;
        }
        let snapshot = Snapshot {
            executor: executor.clone(),
            prefix: Arc::from(inputs),
            cardinality: self.cardinality.clone(),
            features: novel,
            energy: Energy::default(),
        };
        if self.snapshots.len() < MAX_SNAPSHOTS {
            self.snapshots.push(snapshot);
            return;
        }
        // Replace the weakest snapshot, unless it is the one the current run started from.
        let current = match self.current {
            Start::Snapshot(index) => Some(index),
            Start::Setup => None,
        };
        let weakest = self
            .snapshots
            .iter()
            .enumerate()
            .filter(|(index, _)| current != Some(*index))
            .min_by(|(_, a), (_, b)| self.weight(a).total_cmp(&self.weight(b)))
            .map(|(index, _)| index);
        if let Some(index) = weakest {
            self.snapshots[index] = snapshot;
        }
    }

    /// Credits the run's starting state when the run found new coverage or a new state.
    pub(super) fn finish_run(&mut self, new_coverage: bool) {
        if !(new_coverage || self.run_found) {
            return;
        }
        match self.current {
            Start::Setup => self.setup.finds += 1,
            Start::Snapshot(index) => {
                if let Some(snapshot) = self.snapshots.get_mut(index) {
                    snapshot.energy.finds += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_evm_core::evm::EthEvmNetwork;
    use revm::state::EvmStorageSlot;

    fn write(slot: U256, original: U256, present: U256) -> AddressMap<Account> {
        let mut account = Account::default();
        let mut storage = EvmStorageSlot::new(original, Default::default());
        storage.present_value = present;
        account.storage.insert(slot, storage);
        AddressMap::from_iter([(Address::repeat_byte(0x11), account)])
    }

    fn snapshots() -> StateSnapshots<EthEvmNetwork> {
        StateSnapshots::new(10)
    }

    #[test]
    fn small_value_sets_stay_exact_then_bin_by_magnitude() {
        let mut snapshots = snapshots();
        let slot = U256::from(3);
        for value in 1..=MAX_SET_VALUES as u64 {
            assert!(!snapshots.observe(&write(slot, U256::ZERO, U256::from(value))).is_empty());
        }
        // Past the set limit the slot is a distribution: 1000 and 1001 share a bin.
        assert!(!snapshots.observe(&write(slot, U256::ZERO, U256::from(1000))).is_empty());
        assert!(snapshots.observe(&write(slot, U256::ZERO, U256::from(1001))).is_empty());
        assert!(!snapshots.observe(&write(slot, U256::ZERO, U256::from(1 << 20))).is_empty());
    }

    #[test]
    fn mapping_entries_count_by_log2_cardinality() {
        let mut snapshots = snapshots();
        let entry = |i: u64| U256::MAX - U256::from(i);
        // First entry: new value and new cardinality bin.
        assert!(!snapshots.observe(&write(entry(0), U256::ZERO, U256::ONE)).is_empty());
        // Second entry: same value, but the count moves to the next bin.
        assert!(!snapshots.observe(&write(entry(1), U256::ZERO, U256::ONE)).is_empty());
        // Third entry: count 3 shares the bin of 2.
        assert!(snapshots.observe(&write(entry(2), U256::ZERO, U256::ONE)).is_empty());
        assert!(!snapshots.observe(&write(entry(3), U256::ZERO, U256::ONE)).is_empty());
    }

    #[test]
    fn unchanged_and_cheatcode_writes_are_ignored() {
        let mut snapshots = snapshots();
        assert!(snapshots.observe(&write(U256::ZERO, U256::ONE, U256::ONE)).is_empty());
        let mut changeset = write(U256::ZERO, U256::ZERO, U256::ONE);
        let account = changeset.remove(&Address::repeat_byte(0x11)).unwrap();
        changeset.insert(CHEATCODE_ADDRESS, account);
        assert!(snapshots.observe(&changeset).is_empty());
    }

    #[test]
    fn log2_bins_fold_sign() {
        assert_eq!(log2_bin(U256::ZERO), 0);
        assert_eq!(log2_bin(U256::from(1)), 1);
        assert_eq!(log2_bin(U256::from(255)), 8);
        assert_eq!(log2_bin(U256::MAX), 512);
        assert_eq!(log2_bin(U256::MAX - U256::from(255)), 504);
        assert!(log2_bin(U256::MAX) > log2_bin(U256::from(1) << 254));
    }
}
