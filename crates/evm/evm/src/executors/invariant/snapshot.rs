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
    map::{AddressMap, HashMap, HashSet},
};
use foundry_evm_core::{constants::CHEATCODE_ADDRESS, evm::FoundryEvmNetwork};
use foundry_evm_fuzz::BasicTxDetails;
use proptest::{prelude::Rng, test_runner::TestRunner};
use revm::state::Account;
use std::{
    hash::{BuildHasher, BuildHasherDefault, DefaultHasher},
    sync::Arc,
};

/// Most snapshots kept per worker.
const MAX_SNAPSHOTS: usize = 64;
/// Storage slots that the snapshots of all workers may copy together. Each snapshot holds a full
/// copy of the backend, so workers keep fewer snapshots as their states grow, but at least two, so
/// a run resumed from one snapshot can still replace the other.
const SNAPSHOT_SLOT_BUDGET: usize = 1 << 20;
/// Snapshots are only taken while the reproducing sequence is at most this many run depths long.
const MAX_PREFIX_DEPTHS: usize = 4;
/// Distinct values a slot may take before it is treated as a distribution rather than a set.
const MAX_SET_VALUES: usize = 64;
/// Least share of its find rate that a state keeps when every call made from it reverts.
const MIN_LIVENESS: f64 = 0.01;

/// Slots at or above 2^128 are treated as computed (mapping or dynamic array) slots.
const COMPUTED_SLOT: U256 = U256::from_limbs([0, 0, 1, 0]);

/// Sign-folded bit length: small positive and small negative (two's complement) values land in
/// different, magnitude-ordered bins.
fn log2_bin(value: U256) -> u16 {
    if value.bit(255) { 512 - (!value).bit_len() as u16 } else { value.bit_len() as u16 }
}

/// Storage slots held by the executor's backend, and its nonzero computed slots per contract.
fn storage_stats<FEN: FoundryEvmNetwork>(executor: &Executor<FEN>) -> (usize, AddressMap<u64>) {
    let backend = executor.backend();
    let mem_db = &backend.mem_db().cache.accounts;
    let fork_db = backend.active_fork_db().map(|db| &db.cache.accounts);
    // Cloning the backend copies both databases.
    let slots = mem_db
        .values()
        .chain(fork_db.into_iter().flat_map(|accounts| accounts.values()))
        .map(|account| account.storage.len())
        .sum();
    // Calls only read and write the active database, which holds a copy of the persistent
    // accounts, so count entries there alone.
    let cardinality = fork_db
        .unwrap_or(mem_db)
        .iter()
        .filter_map(|(address, account)| {
            let computed = account
                .storage
                .iter()
                .filter(|(slot, value)| **slot >= COMPUTED_SLOT && !value.is_zero())
                .count() as u64;
            (computed > 0).then_some((*address, computed))
        })
        .collect();
    (slots, cardinality)
}

#[derive(Clone, Copy, Default)]
struct Energy {
    picks: u32,
    finds: u32,
    /// Calls made by the runs started from this state, and how many of them reverted.
    calls: u64,
    reverted: u64,
}

impl Energy {
    /// How often runs from this state find new coverage or a new state, scaled down by how
    /// often their calls revert.
    ///
    /// A state where most handlers revert, such as a protocol an admin call has paused, has few
    /// reachable successors, so runs resumed from it spend their calls on reverts. The squared
    /// success rate makes that penalty steep while [`MIN_LIVENESS`] keeps every state in the
    /// schedule.
    fn productivity(self) -> f64 {
        let finds = f64::from(self.finds + 1) / f64::from(self.picks + 1);
        if self.calls == 0 {
            return finds;
        }
        let success = 1.0 - self.reverted as f64 / self.calls as f64;
        finds * (success * success).max(MIN_LIVENESS)
    }
}

struct Snapshot<FEN: FoundryEvmNetwork> {
    executor: Executor<FEN>,
    prefix: Arc<[BasicTxDetails]>,
    /// Nonzero computed slots per contract in this state.
    cardinality: AddressMap<u64>,
    /// Storage slots held by the executor's backend.
    slots: usize,
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
    /// A feature first reached by the prefix's last call, identifying the state it produced.
    pub state: Option<u64>,
}

/// Per-worker pool of snapshots and the state features seen so far.
pub(super) struct StateSnapshots<FEN: FoundryEvmNetwork> {
    /// Storage slots that this worker's snapshots may copy together.
    slot_budget: usize,
    max_prefix: usize,
    /// Hashes features with fixed keys, so a seed picks the same snapshots in every process.
    hasher: BuildHasherDefault<DefaultHasher>,
    /// Feature -> number of calls that reached it.
    hits: HashMap<u64, u32>,
    /// Distinct values per (contract, slot) until the slot becomes a distribution.
    slot_values: HashMap<(Address, U256), HashSet<U256>>,
    setup: Energy,
    snapshots: Vec<Snapshot<FEN>>,
    current: Start,
    /// Nonzero computed slots per contract in the post-setUp state.
    setup_cardinality: AddressMap<u64>,
    /// Nonzero computed slots per contract in the current run.
    cardinality: AddressMap<u64>,
    /// Whether the current run reached a new feature.
    run_found: bool,
}

impl<FEN: FoundryEvmNetwork> StateSnapshots<FEN> {
    /// Creates the pool of one of `workers` workers, whose runs start from `setup`.
    pub(super) fn new(depth: u32, setup: &Executor<FEN>, workers: usize) -> Self {
        Self {
            slot_budget: SNAPSHOT_SLOT_BUDGET / workers.max(1),
            max_prefix: (depth as usize).saturating_mul(MAX_PREFIX_DEPTHS),
            hasher: BuildHasherDefault::default(),
            hits: HashMap::default(),
            slot_values: HashMap::default(),
            setup: Energy::default(),
            snapshots: Vec::new(),
            current: Start::Setup,
            setup_cardinality: storage_stats(setup).1,
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
            Start::Setup => self.start_from_setup(setup),
            Start::Snapshot(index) => {
                let snapshot = &mut self.snapshots[index];
                snapshot.energy.picks += 1;
                self.cardinality.clone_from(&snapshot.cardinality);
                RunStart {
                    executor: snapshot.executor.clone(),
                    prefix: snapshot.prefix.clone(),
                    state: snapshot.features.first().copied(),
                }
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

    /// Starts a candidate replay from setup without crediting the previous run's snapshot.
    pub(super) fn start_from_setup(&mut self, setup: &Executor<FEN>) -> RunStart<FEN> {
        self.run_found = false;
        self.current = Start::Setup;
        self.setup.picks += 1;
        self.cardinality.clone_from(&self.setup_cardinality);
        RunStart { executor: setup.clone(), prefix: Arc::from([]), state: None }
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
            // Mapping keys and array indices are not part of the state's shape: fold them. A
            // slot's set can fill up mid-call, so visit writes by folded slot and value, not in
            // storage map order or by keys, which can be random addresses.
            let mut writes = account
                .storage
                .iter()
                .filter(|(_, value)| value.is_changed())
                .map(|(slot, value)| {
                    let computed = *slot >= COMPUTED_SLOT;
                    (if computed { U256::MAX } else { *slot }, computed, value)
                })
                .collect::<Vec<_>>();
            writes.sort_unstable_by_key(|(slot_key, _, value)| (*slot_key, value.present_value));
            for (slot_key, computed, value) in writes {
                let present = value.present_value;
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
        let slots = storage_stats(executor).0;
        let mut total = self.snapshots.iter().map(|snapshot| snapshot.slots).sum::<usize>();
        // Evict the weakest snapshots, except the one the current run started from, until the new
        // one fits the pool and its slot budget.
        while self.snapshots.len() >= 2
            && (self.snapshots.len() >= MAX_SNAPSHOTS || total + slots > self.slot_budget)
        {
            let current = match self.current {
                Start::Snapshot(index) => Some(index),
                Start::Setup => None,
            };
            let Some(weakest) = self
                .snapshots
                .iter()
                .enumerate()
                .filter(|(index, _)| current != Some(*index))
                .min_by(|(_, a), (_, b)| self.weight(a).total_cmp(&self.weight(b)))
                .map(|(index, _)| index)
            else {
                break;
            };
            total -= self.snapshots.remove(weakest).slots;
            if let Start::Snapshot(index) = &mut self.current
                && *index > weakest
            {
                *index -= 1;
            }
        }
        self.snapshots.push(Snapshot {
            executor: executor.clone(),
            prefix: Arc::from(inputs),
            cardinality: self.cardinality.clone(),
            slots,
            features: novel,
            energy: Energy::default(),
        });
    }

    /// Records the run's calls and reverts against its starting state, and credits the state
    /// when the run found new coverage or a new state.
    pub(super) fn finish_run(&mut self, new_coverage: bool, calls: u64, reverted: u64) {
        let energy = match self.current {
            Start::Setup => &mut self.setup,
            Start::Snapshot(index) => match self.snapshots.get_mut(index) {
                Some(snapshot) => &mut snapshot.energy,
                None => return,
            },
        };
        energy.calls += calls;
        energy.reverted += reverted;
        if new_coverage || self.run_found {
            energy.finds += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executors::ExecutorBuilder;
    use alloy_evm::EvmEnv;
    use alloy_primitives::Bytes;
    use foundry_evm_core::{
        backend::{Backend, DatabaseExt},
        evm::EthEvmNetwork,
        fork::CreateFork,
        opts::EvmOpts,
    };
    use foundry_evm_fuzz::CallDetails;
    use revm::{
        context::{JournalInner, TxEnv},
        state::EvmStorageSlot,
    };

    fn write(slot: U256, original: U256, present: U256) -> AddressMap<Account> {
        let mut account = Account::default();
        let mut storage = EvmStorageSlot::new(original, Default::default());
        storage.present_value = present;
        account.storage.insert(slot, storage);
        AddressMap::from_iter([(Address::repeat_byte(0x11), account)])
    }

    fn executor() -> Executor<EthEvmNetwork> {
        let backend = Backend::spawn(None).unwrap();
        ExecutorBuilder::default().build(
            EvmEnv::default(),
            TxEnv::default(),
            backend,
            Default::default(),
        )
    }

    fn snapshots() -> StateSnapshots<EthEvmNetwork> {
        StateSnapshots::new(10, &executor(), 1)
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
    fn reverting_runs_lower_a_states_productivity() {
        let live = Energy { picks: 4, finds: 2, calls: 400, reverted: 40 };
        let dead = Energy { picks: 4, finds: 2, calls: 400, reverted: 360 };
        let untried = Energy { picks: 0, finds: 0, calls: 0, reverted: 0 };
        assert!(dead.productivity() < live.productivity());
        let expected_live = 0.6 * 0.81;
        let expected_dead = 0.6 * MIN_LIVENESS;
        assert!((live.productivity() - expected_live).abs() < 1e-9);
        assert!((dead.productivity() - expected_dead).abs() < 1e-9);
        assert_eq!(untried.productivity(), 1.0);
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

    #[test]
    fn deletions_count_from_setup_entries() {
        let entry = |i: u64| U256::MAX - U256::from(i);
        let mut setup = executor();
        for i in 0..4 {
            setup.set_storage_slot(Address::repeat_byte(0x11), entry(i), U256::ONE).unwrap();
        }
        let mut snapshots = StateSnapshots::new(10, &setup, 1);
        snapshots.start(&mut TestRunner::deterministic(), &setup);
        // 4 -> 3 entries: a new value and a new cardinality bin.
        assert!(!snapshots.observe(&write(entry(0), U256::ONE, U256::ZERO)).is_empty());
        // 3 -> 2 entries share a bin.
        assert!(snapshots.observe(&write(entry(1), U256::ONE, U256::ZERO)).is_empty());
        // 2 -> 1 and 1 -> 0 entries each reach a new bin.
        assert!(!snapshots.observe(&write(entry(2), U256::ONE, U256::ZERO)).is_empty());
        assert!(!snapshots.observe(&write(entry(3), U256::ONE, U256::ZERO)).is_empty());
    }

    #[test]
    fn snapshot_pool_stays_within_slot_budget() {
        let tx = BasicTxDetails {
            warp: None,
            roll: None,
            sender: Address::ZERO,
            call_details: CallDetails {
                target: Address::ZERO,
                calldata: Bytes::new(),
                value: None,
            },
        };
        let retained = |pool: &StateSnapshots<EthEvmNetwork>| {
            pool.snapshots.iter().map(|snapshot| storage_stats(&snapshot.executor).0).sum::<usize>()
        };

        // Runs that keep adding storage: the snapshots never copy more than the worker's share
        // of the budget.
        let mut state = executor();
        let mut pool = StateSnapshots::new(10, &state, 16);
        for step in 0..16u64 {
            for slot in 0..1u64 << 11 {
                let slot = U256::from((step << 11) + slot);
                state.set_storage_slot(Address::repeat_byte(0x22), slot, U256::ONE).unwrap();
            }
            pool.capture(&state, std::slice::from_ref(&tx), vec![step]);
            assert!(retained(&pool) <= SNAPSHOT_SLOT_BUDGET / 16);
            // Deeper states evict weaker ones rather than being declined.
            assert_eq!(pool.snapshots.last().unwrap().features, [step]);
        }

        // Every worker keeps at least two snapshots, even past its budget.
        let mut pool = StateSnapshots::new(10, &state, 64);
        for feature in 1..=3 {
            pool.capture(&state, std::slice::from_ref(&tx), vec![feature]);
        }
        assert_eq!(pool.snapshots.len(), 2);
        // A run resumed from a snapshot keeps it and replaces the other one.
        pool.current = Start::Snapshot(0);
        let start = pool.snapshots[0].features.clone();
        pool.capture(&state, std::slice::from_ref(&tx), vec![4]);
        assert_eq!(pool.snapshots[0].features, start);
        assert_eq!(pool.snapshots[1].features, [4]);
    }

    #[test]
    fn features_do_not_depend_on_storage_order_or_mapping_keys() {
        let entry = |i: u64| U256::MAX - U256::from(i);
        let outcomes = (0..16)
            .map(|round| {
                let mut snapshots = snapshots();
                for i in 1..MAX_SET_VALUES as u64 {
                    snapshots.observe(&write(entry(100 + i), U256::ZERO, U256::from(i)));
                }
                // One call fills the folded entry set: which value stays exact depends on the
                // order the two writes are visited in. Alternate which key holds which value, as
                // with keys derived from random senders.
                let mut account = Account::default();
                let keys = if round % 2 == 0 { [1, 2] } else { [2, 1] };
                for (i, value) in keys.into_iter().zip([U256::from(1000), U256::from(1u64 << 40)]) {
                    let mut storage = EvmStorageSlot::new(U256::ZERO, Default::default());
                    storage.present_value = value;
                    account.storage.insert(entry(i), storage);
                }
                let filled = snapshots
                    .observe(&AddressMap::from_iter([(Address::repeat_byte(0x11), account)]));
                let next =
                    snapshots.observe(&write(entry(3), U256::ZERO, U256::from((1u64 << 40) + 1)));
                (filled, next)
            })
            .collect::<Vec<_>>();
        assert!(outcomes.windows(2).all(|pair| pair[0] == pair[1]), "{outcomes:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_entries_count_from_the_active_database() {
        let entry = |i: u64| U256::MAX - U256::from(i);
        let mut setup = executor();
        let persistent = Address::repeat_byte(0x11);
        for i in 0..4 {
            setup.set_storage_slot(persistent, entry(i), U256::ONE).unwrap();
        }
        let backend = setup.backend_mut();
        backend.add_persistent_account(persistent);
        let evm_opts = EvmOpts {
            fork_url: Some(foundry_test_utils::rpc::next_http_rpc_endpoint()),
            ..Default::default()
        };
        backend
            .create_select_fork(
                CreateFork {
                    enable_caching: false,
                    url: evm_opts.fork_url.clone().unwrap(),
                    evm_opts,
                },
                &mut EvmEnv::default(),
                &mut TxEnv::default(),
                &mut JournalInner::new(),
            )
            .unwrap();

        // Selecting the fork copied the persistent account, so both databases hold its entries.
        let (slots, cardinality) = storage_stats(&setup);
        assert_eq!(slots, 8);
        assert_eq!(cardinality, AddressMap::from_iter([(persistent, 4)]));
    }
}
