use crate::{
    BasicTxDetails, Fuzzer,
    invariant::{
        FuzzRunIdentifiedContracts, TargetedContract, TargetedContractEvent, TargetedContracts,
    },
    strategies::literals::LiteralsDictionary,
};
use alloy_dyn_abi::{DynSolType, DynSolValue, EventExt, FunctionExt};
use alloy_json_abi::Function;
use alloy_primitives::{
    Address, B256, Bytes, Log, U256,
    map::{AddressIndexSet, AddressMap, B256IndexSet, HashMap, HashSet, IndexSet},
};
use foundry_common::{
    ignore_metadata_hash,
    mapping_slots::MappingSlots,
    slot_identifier::{SlotIdentifier, SlotInfo},
};
use foundry_config::FuzzDictionaryConfig;
use foundry_evm_core::{
    bytecode::InstIter, eip2935::is_history_storage_address, utils::StateChangeset,
};
use revm::{
    database::{CacheDB, DatabaseRef, DbAccount},
    state::AccountInfo,
};
use std::{cell::RefCell, fmt, rc::Rc, sync::Arc};

#[cfg(test)]
use revm::database::InMemoryDB;

/// The maximum number of bytes we will look at in bytecodes to find push bytes (24 KiB).
///
/// This is to limit the performance impact of fuzz tests that might deploy arbitrarily sized
/// bytecode (as is the case with Solmate).
const PUSH_BYTE_ANALYSIS_LIMIT: usize = 24 * 1024;

/// Immutable fuzz dictionary seed used by parallel stateless fuzz workers.
#[derive(Clone, Debug)]
pub struct EvmFuzzState {
    inner: Arc<FuzzDictionary>,
    /// Addresses of external libraries deployed in test setup, excluded from fuzz test inputs.
    pub deployed_libs: Vec<Address>,
}

/// Worker-local fuzz state.
///
/// Stateless workers share the immutable campaign seed, while invariant workers own an isolated
/// mutable dictionary. The latter deliberately uses `Rc<RefCell<_>>`: worker state is not shared
/// across threads.
#[derive(Clone, Debug)]
pub struct FuzzState {
    inner: FuzzStateInner,
    /// Addresses of external libraries deployed in test setup, excluded from fuzz test inputs.
    pub deployed_libs: Vec<Address>,
}

#[derive(Clone, Debug)]
enum FuzzStateInner {
    Stateless(Arc<FuzzDictionary>),
    Invariant(Rc<RefCell<FuzzDictionary>>),
}

pub(crate) trait DictionaryRead: Clone + 'static {
    fn deployed_libs(&self) -> &[Address];
    fn with_dictionary<R>(&self, f: impl FnOnce(&FuzzDictionary) -> R) -> R;
}

impl EvmFuzzState {
    #[cfg(test)]
    pub(crate) fn test() -> Self {
        Self::new(&[], &InMemoryDB::default(), FuzzDictionaryConfig::default(), None)
    }

    pub fn new<DB: DatabaseRef>(
        deployed_libs: &[Address],
        db: &CacheDB<DB>,
        config: FuzzDictionaryConfig,
        literals: Option<&LiteralsDictionary>,
    ) -> Self {
        // Sort accounts to ensure deterministic dictionary generation from the same setUp state.
        let mut accs = db.cache.accounts.iter().collect::<Vec<_>>();
        accs.sort_by_key(|(address, _)| *address);

        // Create fuzz dictionary and insert values from db state.
        let mut dictionary = FuzzDictionary::new(config);
        dictionary.insert_db_values(accs);
        if let Some(literals) = literals {
            dictionary.literal_values = literals.clone();
        }

        Self { inner: Arc::new(dictionary), deployed_libs: deployed_libs.to_vec() }
    }

    pub fn stateless_worker(&self) -> FuzzState {
        FuzzState {
            inner: FuzzStateInner::Stateless(Arc::clone(&self.inner)),
            deployed_libs: self.deployed_libs.clone(),
        }
    }

    pub fn into_invariant(self) -> FuzzState {
        FuzzState {
            inner: FuzzStateInner::Invariant(Rc::new(RefCell::new((*self.inner).clone()))),
            deployed_libs: self.deployed_libs,
        }
    }

    pub fn fork(&self) -> Self {
        Self { inner: Arc::clone(&self.inner), deployed_libs: self.deployed_libs.clone() }
    }

    pub fn collect_values(&mut self, values: impl IntoIterator<Item = B256>) {
        let dict = Arc::make_mut(&mut self.inner);
        for value in values {
            dict.insert_value(value);
        }
    }

    /// Logs stats about the current state.
    pub fn log_stats(&self) {
        self.inner.log_stats();
    }

    /// Test-only helper to seed the dictionary with literal values.
    #[cfg(test)]
    pub(crate) fn seed_literals(&mut self, map: super::LiteralMaps) {
        Arc::make_mut(&mut self.inner).seed_literals(map);
    }
}

impl FuzzState {
    pub fn snapshot(&self) -> EvmFuzzState {
        EvmFuzzState {
            inner: Arc::new(self.with_dictionary(Clone::clone)),
            deployed_libs: self.deployed_libs.clone(),
        }
    }

    pub fn collect_values(&self, values: impl IntoIterator<Item = B256>) {
        let FuzzStateInner::Invariant(inner) = &self.inner else { return };
        let mut dict = inner.borrow_mut();
        for value in values {
            dict.insert_value(value);
        }
    }

    pub fn collect_fuzzer_values(&self, fuzzer: &mut Fuzzer) {
        if fuzzer.collected_values.is_empty() {
            return;
        }

        let FuzzStateInner::Invariant(inner) = &self.inner else { return };
        let mut dict = inner.borrow_mut();
        for value in fuzzer.collected_values.drain(..) {
            dict.insert_value(value);
        }
    }

    /// Collects state changes from a [StateChangeset] and logs into a worker state
    /// according to the given [FuzzDictionaryConfig].
    #[allow(clippy::too_many_arguments)]
    pub fn collect_values_from_call(
        &self,
        fuzzed_contracts: &FuzzRunIdentifiedContracts,
        tx: &BasicTxDetails,
        result: &Bytes,
        logs: &[Log],
        state_changeset: &StateChangeset,
        run_depth: u32,
        mapping_slots: Option<&AddressMap<MappingSlots>>,
    ) {
        if logs.is_empty() && result.is_empty() && state_changeset.is_empty() {
            return;
        }

        let FuzzStateInner::Invariant(inner) = &self.inner else { return };
        let mut dict = inner.borrow_mut();
        let targets = fuzzed_contracts.targets();
        let (target_contract, target_function) = if logs.is_empty() && result.is_empty() {
            (None, None)
        } else {
            targets.fuzzed_artifacts(tx)
        };
        if !logs.is_empty() {
            dict.insert_logs_values(target_contract, logs, run_depth);
        }
        if !result.is_empty() {
            dict.insert_result_values(target_function, result, run_depth);
        }
        dict.insert_new_state_values(state_changeset, &targets, mapping_slots);
    }

    /// Collects typed trace-cmp operands from sancov-instrumented code.
    /// Values are inserted into both persistent state values (survive reverts) and typed
    /// sample buckets (for ABI-aware mutation).
    pub fn collect_typed_cmp_values(&self, values: impl IntoIterator<Item = (u8, B256)>) {
        let FuzzStateInner::Invariant(inner) = &self.inner else { return };
        let mut dict = inner.borrow_mut();
        for (width, value) in values {
            dict.insert_persistent_value(value);
            dict.insert_typed_cmp_value(width, value);
        }
    }

    /// Removes entries that were not promoted into the persistent dictionary prefix.
    ///
    /// Should be called between invariant runs to discard unselected feedback.
    pub fn revert(&self) {
        if let FuzzStateInner::Invariant(inner) = &self.inner {
            inner.borrow_mut().revert();
        }
    }

    /// Logs stats about the current state.
    pub fn log_stats(&self) {
        self.with_dictionary(FuzzDictionary::log_stats);
    }

    pub fn deployed_libs(&self) -> &[Address] {
        &self.deployed_libs
    }

    pub fn with_dictionary<R>(&self, f: impl FnOnce(&FuzzDictionary) -> R) -> R {
        match &self.inner {
            FuzzStateInner::Stateless(inner) => f(inner),
            FuzzStateInner::Invariant(inner) => f(&inner.borrow()),
        }
    }

    /// Retains bounded raw values and addresses from an invariant run with new coverage.
    pub fn promote_ephemeral_values(&self) {
        if let FuzzStateInner::Invariant(inner) = &self.inner {
            inner.borrow_mut().promote_ephemeral_values();
        }
    }
}

impl DictionaryRead for FuzzState {
    fn deployed_libs(&self) -> &[Address] {
        self.deployed_libs()
    }

    fn with_dictionary<R>(&self, f: impl FnOnce(&FuzzDictionary) -> R) -> R {
        self.with_dictionary(f)
    }
}

impl DictionaryRead for EvmFuzzState {
    fn deployed_libs(&self) -> &[Address] {
        &self.deployed_libs
    }

    fn with_dictionary<R>(&self, f: impl FnOnce(&FuzzDictionary) -> R) -> R {
        f(&self.inner)
    }
}

impl From<EvmFuzzState> for FuzzState {
    fn from(state: EvmFuzzState) -> Self {
        state.into_invariant()
    }
}

// We're using `IndexSet` to have a stable element order when restoring persisted state, as well as
// for performance when iterating over the sets.
/// Maximum number of persistent values from sancov trace-cmp.
const MAX_PERSISTENT_VALUES: usize = 2048;
/// Maximum additional state values retained from coverage-producing invariant runs.
const MAX_COVERAGE_VALUES: usize = 4096;
/// Maximum additional addresses retained from coverage-producing invariant runs.
const MAX_COVERAGE_ADDRESSES: usize = 512;
/// Maximum cached storage slot layout lookups per fuzz dictionary.
const MAX_SLOT_INFO_CACHE_ENTRIES: usize = 4096;

#[derive(Clone)]
pub struct FuzzDictionary {
    /// Collected state values.
    state_values: B256IndexSet,
    /// Addresses that already had their PUSH bytes collected.
    addresses: AddressIndexSet,
    /// Code hashes that already had their PUSH bytes collected.
    push_bytecode_hashes: B256IndexSet,
    /// Configuration for the dictionary.
    config: FuzzDictionaryConfig,
    /// Length of the retained state-value prefix, initially seeded from the database.
    db_state_values: usize,
    /// Length of the retained address prefix, initially seeded from the database.
    db_addresses: usize,
    /// Number of bytecode hashes initially collected from db.
    /// Used to revert new collected bytecode hashes at the end of each run.
    db_push_bytecode_hashes: usize,
    /// Typed runtime sample values persisted across invariant runs.
    /// Initially seeded with literal values collected from the source code.
    sample_values: HashMap<DynSolType, B256IndexSet>,
    /// Lazily initialized dictionary of literal values collected from the source code.
    literal_values: LiteralsDictionary,
    /// Tracks whether literals from `literal_values` have been merged into `sample_values`.
    ///
    /// Set to `true` on first call to `seed_samples()`. Before seeding, `samples()` checks both
    /// maps separately. After seeding, literals are merged in, so only `sample_values` is checked.
    samples_seeded: bool,
    /// Persistent values from sancov trace-cmp that survive `revert()` across runs.
    persistent_values: B256IndexSet,
    /// Additional state values retained from coverage-producing runs.
    coverage_values: usize,
    /// Additional addresses retained from coverage-producing runs.
    coverage_addresses: usize,
    /// Parsed storage layout identifiers keyed by the layout allocation.
    slot_identifiers: HashMap<usize, SlotIdentifier>,
    /// Cached non-mapping storage slot identification keyed by layout allocation and slot.
    slot_info_cache: HashMap<(usize, B256), Option<SlotInfo>>,

    misses: usize,
    hits: usize,
}

impl fmt::Debug for FuzzDictionary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FuzzDictionary")
            .field("state_values", &self.state_values.len())
            .field("addresses", &self.addresses)
            .field("persistent_values", &self.persistent_values.len())
            .finish()
    }
}

impl Default for FuzzDictionary {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

impl FuzzDictionary {
    pub fn new(mut config: FuzzDictionaryConfig) -> Self {
        config.max_fuzz_dictionary_values = config.max_fuzz_dictionary_values.max(1);
        let mut dictionary = Self {
            config,
            samples_seeded: false,

            state_values: Default::default(),
            addresses: Default::default(),
            push_bytecode_hashes: Default::default(),
            db_state_values: Default::default(),
            db_addresses: Default::default(),
            db_push_bytecode_hashes: Default::default(),
            sample_values: Default::default(),
            literal_values: Default::default(),
            persistent_values: Default::default(),
            coverage_values: 0,
            coverage_addresses: 0,
            slot_identifiers: Default::default(),
            slot_info_cache: Default::default(),
            misses: Default::default(),
            hits: Default::default(),
        };
        // Zero is a useful default seed even before state or literals populate the dictionary.
        dictionary.insert_value(B256::ZERO);
        dictionary
    }

    /// Seeds `sample_values` with all words from the [`LiteralsDictionary`].
    /// Should only be called once per dictionary lifetime.
    #[cold]
    fn seed_samples(&mut self) {
        trace!("seeding `sample_values` from literal dictionary");
        self.sample_values
            .extend(self.literal_values.get().words.iter().map(|(k, v)| (k.clone(), v.clone())));
        self.samples_seeded = true;
    }

    /// Insert values from initial db state into fuzz dictionary.
    /// These values are persisted across invariant runs.
    fn insert_db_values(&mut self, db_state: Vec<(&Address, &DbAccount)>) {
        for (address, account) in db_state {
            if is_history_storage_address(address) {
                continue;
            }

            // Insert basic account information
            self.insert_value(address.into_word());
            // Insert push bytes
            self.insert_push_bytes_values(address, &account.info);
            // Insert storage values.
            if self.config.include_storage {
                // Sort storage values before inserting to ensure deterministic dictionary.
                let mut values = account.storage.iter().collect::<Vec<_>>();
                values.sort_unstable_by_key(|(slot, _)| **slot);
                for (slot, value) in values {
                    self.insert_storage_value(slot, value, None);
                }
            }
        }

        // We need at least some state data if DB is empty,
        // otherwise we can't select random data for state fuzzing.
        if self.values().is_empty() {
            // Prefill with a random address.
            self.insert_value(Address::random().into_word());
        }

        // Record number of values and addresses inserted from db to be used for reverting at the
        // end of each run.
        self.db_state_values = self.state_values.len();
        self.db_addresses = self.addresses.len();
        self.db_push_bytecode_hashes = self.push_bytecode_hashes.len();
    }

    /// Insert values collected from call result into fuzz dictionary.
    fn insert_result_values(
        &mut self,
        function: Option<&Function>,
        result: &Bytes,
        run_depth: u32,
    ) {
        if let Some(function) = function
            && !function.outputs.is_empty()
        {
            // Decode result and collect samples to be used in subsequent fuzz runs.
            if let Ok(decoded_result) = function.abi_decode_output(result) {
                self.insert_sample_values(decoded_result, run_depth);
            }
        }
    }

    /// Insert values from call log topics and data into fuzz dictionary.
    fn insert_logs_values(
        &mut self,
        target_contract: Option<&TargetedContract>,
        logs: &[Log],
        run_depth: u32,
    ) {
        let mut samples = Vec::new();
        // Decode logs with known events and collect samples from indexed fields and event body.
        for log in logs {
            // Try to decode log with events from contract abi.
            let log_decoded = if let Some(contract) = target_contract {
                let matched_events = log
                    .topics()
                    .first()
                    .and_then(|selector| {
                        contract.event_lookup.by_topic(selector, log.topics().len() - 1)
                    })
                    .unwrap_or(&[]);
                Self::decode_log_events(
                    matched_events,
                    contract.event_lookup.anonymous(),
                    log,
                    &mut samples,
                )
            } else {
                false
            };

            // If we weren't able to decode event then we insert raw data in fuzz dictionary.
            if !log_decoded {
                for &topic in log.topics() {
                    self.insert_value(topic);
                }
                let (chunks, rem) = log.data.data.as_chunks::<32>();
                for chunk in chunks {
                    self.insert_value((*chunk).into());
                }
                if !rem.is_empty() {
                    self.insert_value(B256::right_padding_from(rem));
                }
            }
        }

        // Insert samples collected from current call in fuzz dictionary.
        if !samples.is_empty() {
            self.insert_sample_values(samples, run_depth);
        }
    }

    fn decode_log_events(
        matched_events: &[TargetedContractEvent],
        anonymous_events: &[TargetedContractEvent],
        log: &Log,
        samples: &mut Vec<DynSolValue>,
    ) -> bool {
        let mut matched = matched_events.iter().peekable();
        let mut anonymous = anonymous_events.iter().peekable();
        while matched.peek().is_some() || anonymous.peek().is_some() {
            let event = match (matched.peek(), anonymous.peek()) {
                (Some(matched_event), Some(anonymous_event)) => {
                    if matched_event.order() < anonymous_event.order() {
                        matched.next().unwrap()
                    } else {
                        anonymous.next().unwrap()
                    }
                }
                (Some(_), None) => matched.next().unwrap(),
                (None, Some(_)) => anonymous.next().unwrap(),
                (None, None) => unreachable!(),
            };
            if let Ok(decoded_event) = event.event().decode_log(log) {
                samples.extend(decoded_event.indexed);
                samples.extend(decoded_event.body);
                return true;
            }
        }
        false
    }

    /// Insert values from call state changeset into fuzz dictionary.
    /// These values are discarded at run end unless selected for persistence.
    fn insert_new_state_values(
        &mut self,
        state_changeset: &StateChangeset,
        targets: &TargetedContracts,
        mapping_slots: Option<&AddressMap<MappingSlots>>,
    ) {
        for (address, account) in state_changeset {
            if is_history_storage_address(address) {
                continue;
            }

            // Insert basic account information.
            self.insert_value(address.into_word());
            // Insert push bytes.
            self.insert_push_bytes_values(address, &account.info);
            // Insert storage values.
            if self.config.include_storage && !account.storage.is_empty() {
                let slot_identifier_key = targets.get(address).and_then(|contract| {
                    contract.storage_layout.as_ref().map(|layout| {
                        let key = Arc::as_ptr(layout) as usize;
                        self.slot_identifiers
                            .entry(key)
                            .or_insert_with(|| SlotIdentifier::new(Arc::clone(layout)));
                        key
                    })
                });
                trace!(
                    "{address:?} has mapping_slots {}",
                    mapping_slots.is_some_and(|m| m.contains_key(address))
                );
                let mapping_slots = mapping_slots.and_then(|m| m.get(address));
                for (slot, value) in &account.storage {
                    let slot_info = slot_identifier_key.and_then(|key| {
                        let slot = B256::from(*slot);
                        let value_word = B256::from(value.present_value());
                        self.identify_storage_slot(key, slot, mapping_slots)
                            .filter(|slot_info| slot_info.decode(value_word).is_some())
                    });
                    self.insert_storage_value(slot, &value.present_value(), slot_info);
                }
            }
        }
    }

    fn identify_storage_slot(
        &mut self,
        key: usize,
        slot: B256,
        mapping_slots: Option<&MappingSlots>,
    ) -> Option<SlotInfo> {
        if mapping_slots.is_some() {
            return self
                .slot_identifiers
                .get(&key)
                .and_then(|identifier| identifier.identify(&slot, mapping_slots));
        }

        let cache_key = (key, slot);
        if let Some(slot_info) = self.slot_info_cache.get(&cache_key) {
            return slot_info.clone();
        }

        let slot_info =
            self.slot_identifiers.get(&key).and_then(|identifier| identifier.identify(&slot, None));
        if self.slot_info_cache.len() < MAX_SLOT_INFO_CACHE_ENTRIES {
            self.slot_info_cache.insert(cache_key, slot_info.clone());
        }
        slot_info
    }

    /// Insert values from push bytes into fuzz dictionary.
    /// Values are collected only once for a given bytecode.
    /// Newly collected values are discarded at run end unless selected for persistence.
    fn insert_push_bytes_values(&mut self, address: &Address, account_info: &AccountInfo) {
        if !self.config.include_push_bytes {
            return;
        }

        let Some(code) = &account_info.code else {
            return;
        };
        self.insert_address(*address);
        if self.values_full() {
            return;
        }
        if self.push_bytecode_hashes.insert(account_info.code_hash()) {
            self.collect_push_bytes(ignore_metadata_hash(code.original_byte_slice()));
        }
    }

    fn collect_push_bytes(&mut self, code: &[u8]) {
        let len = code.len().min(PUSH_BYTE_ANALYSIS_LIMIT);
        let code = &code[..len];
        let mut seen = HashSet::default();
        for inst in InstIter::new(code) {
            if self.values_full() {
                break;
            }
            // Don't add 0 to the dictionary as it's already present.
            if !inst.immediate.is_empty()
                && let Some(push_value) = U256::try_from_be_slice(inst.immediate)
                && !push_value.is_zero()
            {
                self.insert_push_value_u256(push_value, &mut seen);
            }
        }
    }

    /// Insert values from single storage slot and storage value into fuzz dictionary.
    /// Uses [`SlotIdentifier`] to identify storage slots types.
    fn insert_storage_value(&mut self, slot: &U256, value: &U256, slot_info: Option<SlotInfo>) {
        let slot = B256::from(*slot);
        let value_word = B256::from(*value);

        // Always insert the slot itself
        self.insert_value(slot);

        if let Some(slot_info) = slot_info {
            trace!(?slot_info, "inserting typed storage value");
            if !self.samples_seeded {
                self.seed_samples();
            }
            self.sample_values
                .entry(slot_info.slot_type.dyn_sol_type)
                .or_default()
                .insert(value_word);
        } else {
            self.insert_value_u256(*value);
        }
    }

    /// Insert address into fuzz dictionary.
    /// Newly collected addresses are discarded at run end unless selected for persistence.
    fn insert_address(&mut self, address: Address) {
        if self.addresses.len() < self.config.max_fuzz_dictionary_addresses {
            self.addresses.insert(address);
        }
    }

    /// Insert raw value into fuzz dictionary.
    ///
    /// Newly collected values are discarded at run end unless selected for persistence.
    ///
    /// Returns true if the value was inserted.
    fn insert_value(&mut self, value: B256) -> bool {
        let insert = !self.values_full();
        if insert {
            let new_value = self.state_values.insert(value);
            let counter = if new_value { &mut self.misses } else { &mut self.hits };
            *counter += 1;
        }
        insert
    }

    /// Insert a persistent value that survives `revert()` across invariant runs.
    /// Used for trace-cmp operands that should compound over time.
    fn insert_persistent_value(&mut self, value: B256) {
        if self.persistent_values.len() >= MAX_PERSISTENT_VALUES {
            return;
        }
        if !self.persistent_values.insert(value) {
            return;
        }
        // `revert()` truncates `state_values` down to the first `db_state_values` entries, so the
        // value must be placed inside that prefix; a plain `insert` appends past it and would be
        // truncated at the end of the run.
        match self.state_values.get_index_of(&value) {
            // Already inside the persisted prefix.
            Some(index) if index < self.db_state_values => {}
            // Collected as an ephemeral value earlier in this run: move it into the prefix.
            Some(index) => {
                self.state_values.move_index(index, self.db_state_values);
                self.db_state_values += 1;
            }
            None => {
                self.state_values.shift_insert(self.db_state_values, value);
                self.db_state_values += 1;
            }
        }
    }

    /// Insert a typed trace-cmp value into the `sample_values` map.
    /// Maps sancov width to `DynSolType` buckets and promotes to larger types.
    fn insert_typed_cmp_value(&mut self, width: u8, value: B256) {
        if !self.samples_seeded {
            self.seed_samples();
        }

        const MAX_TYPED_CMP_PER_BUCKET: usize = 1024;

        let native_type = match width {
            8 => DynSolType::Uint(8),
            16 => DynSolType::Uint(16),
            32 => DynSolType::Uint(32),
            64 => DynSolType::Uint(64),
            _ => DynSolType::Uint(256),
        };

        let insert = |map: &mut HashMap<DynSolType, B256IndexSet>, ty: DynSolType, val: B256| {
            let bucket = map.entry(ty).or_default();
            if bucket.len() < MAX_TYPED_CMP_PER_BUCKET {
                bucket.insert(val);
            }
        };

        insert(&mut self.sample_values, native_type, value);

        if width <= 64 {
            insert(&mut self.sample_values, DynSolType::Uint(128), value);
            insert(&mut self.sample_values, DynSolType::Uint(256), value);
            insert(&mut self.sample_values, DynSolType::Int(256), value);
        }
    }

    fn insert_value_u256(&mut self, value: U256) -> bool {
        // Also add the value below and above the push value to the dictionary.
        let one = U256::ONE;
        let mut inserted = self.insert_value(value.into());
        if !self.values_full() {
            inserted |= self.insert_value((value.wrapping_sub(one)).into());
        }
        if !self.values_full() {
            inserted |= self.insert_value((value.wrapping_add(one)).into());
        }
        inserted
    }

    fn insert_push_value_u256(&mut self, value: U256, seen: &mut HashSet<B256>) -> bool {
        // Also add the value below and above the push value to the dictionary.
        let one = U256::ONE;
        let mut inserted = false;
        for value in [value, value.wrapping_sub(one), value.wrapping_add(one)] {
            if self.values_full() {
                break;
            }
            let value = value.into();
            if seen.insert(value) {
                inserted |= self.insert_value(value);
            }
        }
        inserted
    }

    fn values_full(&self) -> bool {
        self.state_values.len() >= self.config.max_fuzz_dictionary_values
    }

    /// Insert sample values that are reused across multiple runs.
    /// The number of samples is limited to invariant run depth.
    /// If collected samples limit is reached then values are inserted as regular values.
    pub fn insert_sample_values(
        &mut self,
        sample_values: impl IntoIterator<Item = DynSolValue>,
        limit: u32,
    ) {
        if !self.samples_seeded {
            self.seed_samples();
        }
        for sample in sample_values {
            if let (Some(sample_type), Some(sample_value)) = (sample.as_type(), sample.as_word()) {
                if let Some(values) = self.sample_values.get_mut(&sample_type) {
                    if values.len() < limit as usize {
                        values.insert(sample_value);
                    } else {
                        // Insert as state value (will be removed at the end of the run).
                        self.insert_value(sample_value);
                    }
                } else {
                    self.sample_values.entry(sample_type).or_default().insert(sample_value);
                }
            }
        }
    }

    pub const fn values(&self) -> &B256IndexSet {
        &self.state_values
    }

    pub fn len(&self) -> usize {
        self.state_values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.state_values.is_empty()
    }

    /// Returns sample values for a given type, checking both runtime samples and literals.
    ///
    /// Before `seed_samples()` is called, checks both `literal_values` and `sample_values`
    /// separately. After seeding, all literal values are merged into `sample_values`.
    #[inline]
    pub fn samples(&self, param_type: &DynSolType) -> Option<&B256IndexSet> {
        // If not seeded yet, return literals
        if !self.samples_seeded {
            return self.literal_values.get().words.get(param_type);
        }

        self.sample_values.get(param_type)
    }

    /// Returns the collected literal strings, triggering initialization if needed.
    #[inline]
    pub fn ast_strings(&self) -> &IndexSet<String> {
        &self.literal_values.get().strings
    }

    /// Returns the collected literal bytes (hex strings), triggering initialization if needed.
    #[inline]
    pub fn ast_bytes(&self) -> &IndexSet<Bytes> {
        &self.literal_values.get().bytes
    }

    #[inline]
    pub const fn addresses(&self) -> &AddressIndexSet {
        &self.addresses
    }

    /// Reverts values, addresses, and scan-cache entries outside their persistent prefixes.
    pub fn revert(&mut self) {
        self.state_values.truncate(self.db_state_values);
        self.addresses.truncate(self.db_addresses);
        self.push_bytecode_hashes.truncate(self.db_push_bytecode_hashes);
    }

    pub fn log_stats(&self) {
        trace!(
            addresses.len = self.addresses.len(),
            sample.len = self.sample_values.len(),
            state.len = self.state_values.len(),
            state.misses = self.misses,
            state.hits = self.hits,
            "FuzzDictionary stats",
        );
    }

    /// Retains this run's raw feedback within the coverage budget.
    /// Setup values and sancov operands have separate retention budgets.
    /// The bytecode scan watermark stays unchanged because a full dictionary can interrupt a scan.
    fn promote_ephemeral_values(&mut self) {
        let values = (self.state_values.len() - self.db_state_values)
            .min(MAX_COVERAGE_VALUES - self.coverage_values);
        self.db_state_values += values;
        self.coverage_values += values;

        let addresses = (self.addresses.len() - self.db_addresses)
            .min(MAX_COVERAGE_ADDRESSES - self.coverage_addresses);
        self.db_addresses += addresses;
        self.coverage_addresses += addresses;
    }

    #[cfg(test)]
    /// Test-only helper to seed the dictionary with literal values.
    pub(crate) fn seed_literals(&mut self, map: super::LiteralMaps) {
        self.literal_values.set(map);
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use alloy_json_abi::{Event, JsonAbi};
    use foundry_evm_core::eip2935::HISTORY_STORAGE_ADDRESS;
    use revm::bytecode::Bytecode;

    fn account_with_code(raw: &'static [u8]) -> AccountInfo {
        AccountInfo::default().with_code(Bytecode::new_raw(Bytes::from_static(raw)))
    }

    #[test]
    fn log_decoding_preserves_anonymous_event_priority() {
        let mut abi = JsonAbi::new();
        let anonymous_event =
            Event::parse("event AEvent(bytes32 indexed topic, uint256 value) anonymous").unwrap();
        let matched_event = Event::parse("event ZEvent(uint256 value)").unwrap();
        let selector = matched_event.selector();
        abi.events.entry(anonymous_event.name.clone()).or_default().push(anonymous_event);
        abi.events.entry(matched_event.name.clone()).or_default().push(matched_event);
        let contract = TargetedContract::new("Target".to_string(), abi);
        let matched_events = contract.event_lookup.by_topic(&selector, 0).unwrap();
        let word: B256 = U256::from(42).into();
        let log = Log::new_unchecked(Address::ZERO, vec![selector], Bytes::from(word));
        let mut samples = Vec::new();

        assert!(FuzzDictionary::decode_log_events(
            matched_events,
            contract.event_lookup.anonymous(),
            &log,
            &mut samples,
        ));

        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0], DynSolValue::FixedBytes(selector, 32));
        assert_eq!(samples[1], DynSolValue::Uint(U256::from(42), 256));
    }

    #[test]
    fn push_byte_collection_stops_when_dictionary_is_full() {
        let mut dictionary = FuzzDictionary::new(FuzzDictionaryConfig {
            max_fuzz_dictionary_values: 3,
            ..Default::default()
        });

        dictionary.collect_push_bytes(&[0x60, 0x01, 0x60, 0x03]);

        assert_eq!(dictionary.len(), 3);
        assert!(dictionary.state_values.contains(&B256::ZERO));
        assert!(dictionary.state_values.contains(&B256::with_last_byte(1)));
        assert!(dictionary.state_values.contains(&B256::with_last_byte(2)));
        assert!(!dictionary.state_values.contains(&B256::with_last_byte(3)));
    }

    #[test]
    fn zero_value_capacity_keeps_only_required_seed() {
        let mut dictionary = FuzzDictionary::new(FuzzDictionaryConfig {
            max_fuzz_dictionary_values: 0,
            ..Default::default()
        });

        assert_eq!(dictionary.config.max_fuzz_dictionary_values, 1);
        assert_eq!(dictionary.state_values.as_slice(), &[B256::ZERO]);
        assert!(!dictionary.insert_value(B256::with_last_byte(1)));
        assert_eq!(dictionary.state_values.as_slice(), &[B256::ZERO]);
    }

    #[test]
    fn duplicate_push_values_in_same_bytecode_are_collected_once() {
        let mut dictionary = FuzzDictionary::default();

        dictionary.collect_push_bytes(&[0x60, 0x01, 0x60, 0x01]);

        assert!(dictionary.state_values.contains(&B256::ZERO));
        assert!(dictionary.state_values.contains(&B256::with_last_byte(1)));
        assert!(dictionary.state_values.contains(&B256::with_last_byte(2)));
        assert_eq!(dictionary.hits, 1);
    }

    #[test]
    fn duplicate_bytecode_push_bytes_are_collected_once() {
        let mut dictionary = FuzzDictionary::default();
        let account = account_with_code(&[0x60, 0x01]);

        dictionary.insert_push_bytes_values(&Address::repeat_byte(0x11), &account);
        let hits_after_first_scan = dictionary.hits;

        dictionary.insert_push_bytes_values(&Address::repeat_byte(0x22), &account);

        assert_eq!(dictionary.push_bytecode_hashes.len(), 1);
        assert_eq!(dictionary.addresses.len(), 2);
        assert_eq!(dictionary.hits, hits_after_first_scan);
    }

    #[test]
    fn same_address_with_new_bytecode_is_scanned_again() {
        let mut dictionary = FuzzDictionary::default();
        let address = Address::repeat_byte(0x22);

        dictionary.insert_push_bytes_values(&address, &account_with_code(&[0x60, 0x01]));
        dictionary.insert_push_bytes_values(&address, &account_with_code(&[0x60, 0x04]));

        assert_eq!(dictionary.addresses.len(), 1);
        assert_eq!(dictionary.push_bytecode_hashes.len(), 2);
        assert!(dictionary.state_values.contains(&B256::with_last_byte(1)));
        assert!(dictionary.state_values.contains(&B256::with_last_byte(4)));
    }

    #[test]
    fn no_code_account_does_not_block_later_push_byte_scan() {
        let mut dictionary = FuzzDictionary::default();
        let address = Address::repeat_byte(0x33);
        let account_without_code = AccountInfo { code: None, ..Default::default() };

        dictionary.insert_push_bytes_values(&address, &account_without_code);

        assert!(!dictionary.addresses.contains(&address));
        assert_eq!(dictionary.push_bytecode_hashes.len(), 0);

        dictionary.insert_push_bytes_values(&address, &account_with_code(&[0x60, 0x04]));

        assert!(dictionary.addresses.contains(&address));
        assert_eq!(dictionary.push_bytecode_hashes.len(), 1);
        assert!(dictionary.state_values.contains(&B256::with_last_byte(4)));
    }

    #[test]
    fn revert_removes_runtime_bytecode_scan_cache() {
        let mut dictionary = FuzzDictionary::default();
        dictionary.db_state_values = dictionary.state_values.len();
        dictionary.db_addresses = dictionary.addresses.len();
        dictionary.db_push_bytecode_hashes = dictionary.push_bytecode_hashes.len();

        let account = account_with_code(&[0x60, 0x01]);
        dictionary.insert_push_bytes_values(&Address::repeat_byte(0x11), &account);
        assert_eq!(dictionary.push_bytecode_hashes.len(), 1);

        dictionary.revert();
        assert_eq!(dictionary.push_bytecode_hashes.len(), 0);

        dictionary.insert_push_bytes_values(&Address::repeat_byte(0x22), &account);
        assert_eq!(dictionary.push_bytecode_hashes.len(), 1);
        assert!(dictionary.state_values.contains(&B256::with_last_byte(1)));
    }

    #[test]
    fn persistent_value_survives_revert() {
        let mut dictionary = FuzzDictionary::default();
        dictionary.db_state_values = dictionary.state_values.len();

        let ephemeral = B256::from(U256::from(0xbeef_u64));
        let persistent = B256::from(U256::from(0xcafe_u64));
        dictionary.insert_value(ephemeral);
        dictionary.insert_persistent_value(persistent);

        dictionary.revert();

        assert!(dictionary.state_values.contains(&persistent));
        assert!(!dictionary.state_values.contains(&ephemeral));
        assert!(dictionary.db_state_values <= dictionary.state_values.len());

        // Values already inside the persisted prefix are left untouched.
        let watermark = dictionary.db_state_values;
        let len = dictionary.state_values.len();
        dictionary.insert_persistent_value(B256::ZERO);
        assert_eq!(dictionary.db_state_values, watermark);
        assert_eq!(dictionary.state_values.len(), len);
    }

    #[test]
    fn persistent_value_promotes_existing_ephemeral() {
        let mut dictionary = FuzzDictionary::default();
        dictionary.db_state_values = dictionary.state_values.len();

        let ephemeral = B256::from(U256::from(0xbeef_u64));
        let value = B256::from(U256::from(0xdead_u64));
        dictionary.insert_value(ephemeral);
        dictionary.insert_value(value);
        dictionary.insert_persistent_value(value);

        dictionary.revert();

        assert!(dictionary.state_values.contains(&value));
        assert!(!dictionary.state_values.contains(&ephemeral));
        assert!(dictionary.db_state_values <= dictionary.state_values.len());
    }

    #[test]
    fn history_storage_account_is_excluded_from_initial_dictionary() {
        let mut db = InMemoryDB::default();
        let code = Bytecode::new_raw(Bytes::from_static(&[0x61, 0x01, 0x23, 0x00]));
        db.insert_account_info(HISTORY_STORAGE_ADDRESS, AccountInfo::default().with_code(code));
        db.insert_account_storage(HISTORY_STORAGE_ADDRESS, U256::from(7), U256::from(0xdead_u64))
            .unwrap();

        let state = EvmFuzzState::new(&[], &db, FuzzDictionaryConfig::default(), None);

        state.with_dictionary(|dict| {
            assert!(!dict.values().contains(&HISTORY_STORAGE_ADDRESS.into_word()));
            assert!(!dict.values().contains(&B256::from(U256::from(0x123))));
            assert!(!dict.values().contains(&B256::with_last_byte(7)));
            assert!(!dict.values().contains(&B256::from(U256::from(0xdead_u64))));
        });
    }

    #[test]
    fn worker_modes_share_seed_but_isolate_feedback() {
        let seed = EvmFuzzState::test();
        let readonly = seed.stateless_worker();
        let first = seed.clone().into_invariant();
        let second = seed.into_invariant();
        let transient = B256::from(U256::from(0xdead_u64));
        let persistent = B256::from(U256::from(0xbeef_u64));

        readonly.collect_values([transient]);
        assert!(!readonly.with_dictionary(|dict| dict.state_values.contains(&transient)));

        first.collect_values([transient]);
        first.collect_typed_cmp_values([(8, persistent)]);
        assert!(first.with_dictionary(|dict| dict.state_values.contains(&transient)));
        assert!(!second.with_dictionary(|dict| dict.state_values.contains(&transient)));

        first.revert();
        assert!(!first.with_dictionary(|dict| dict.state_values.contains(&transient)));
        first.with_dictionary(|dict| {
            assert!(dict.state_values.contains(&persistent));
            assert!(dict.persistent_values.contains(&persistent));
            assert!(dict.sample_values[&DynSolType::Uint(8)].contains(&persistent));
        });
    }

    #[test]
    fn coverage_promotion_retains_feedback_but_not_scan_cache() {
        let mut dictionary = FuzzDictionary::default();
        dictionary.insert_db_values(Vec::new());
        let address = Address::repeat_byte(0x11);
        let account = account_with_code(&[0x61, 0xca, 0xfe]);
        let learned = B256::from(U256::from(0xcafe_u64));
        let transient = B256::from(U256::from(0xdead_u64));
        dictionary.insert_push_bytes_values(&address, &account);

        dictionary.promote_ephemeral_values();
        dictionary.revert();

        assert!(dictionary.values().contains(&B256::ZERO));
        assert!(dictionary.values().contains(&learned));
        assert!(dictionary.addresses().contains(&address));
        assert!(dictionary.push_bytecode_hashes.is_empty());

        // A later run without coverage must not extend the retained prefixes.
        dictionary.insert_value(transient);
        dictionary.insert_address(Address::repeat_byte(0x22));
        dictionary.revert();
        assert!(dictionary.values().contains(&learned));
        assert!(!dictionary.values().contains(&transient));
        assert_eq!(dictionary.addresses().as_slice(), &[address]);

        // A retained address must still allow newly deployed code to be scanned.
        dictionary.insert_push_bytes_values(&address, &account_with_code(&[0x61, 0xbe, 0xef]));
        assert!(dictionary.values().contains(&B256::from(U256::from(0xbeef_u64))));
    }

    #[test]
    fn coverage_promotion_is_bounded_across_runs() {
        let mut dictionary = FuzzDictionary::default();
        dictionary.insert_db_values(Vec::new());
        let baseline_len = dictionary.len();
        for run in 0..6 {
            for offset in 1..=1024 {
                let word = B256::from(U256::from(run * 1024 + offset));
                dictionary.insert_value(word);
                dictionary.insert_address(Address::from_word(word));
            }
            dictionary.promote_ephemeral_values();
            dictionary.revert();
            assert!(dictionary.len() <= baseline_len + MAX_COVERAGE_VALUES);
            assert!(dictionary.addresses().len() <= MAX_COVERAGE_ADDRESSES);
        }
        assert_eq!(dictionary.len(), baseline_len + MAX_COVERAGE_VALUES);
        assert_eq!(dictionary.addresses().len(), MAX_COVERAGE_ADDRESSES);
        assert!(!dictionary.values().contains(&B256::from(U256::from(6144))));
    }

    #[test]
    fn coverage_promotion_respects_configured_capacity() {
        for capacity in [0, 1, 8] {
            let mut dictionary = FuzzDictionary::new(FuzzDictionaryConfig {
                max_fuzz_dictionary_values: capacity,
                max_fuzz_dictionary_addresses: capacity,
                ..Default::default()
            });
            dictionary.insert_db_values(Vec::new());
            for value in 1..=16 {
                let word = B256::from(U256::from(value));
                dictionary.insert_value(word);
                dictionary.insert_address(Address::from_word(word));
            }

            dictionary.promote_ephemeral_values();
            dictionary.revert();

            assert_eq!(dictionary.len(), capacity.max(1));
            assert_eq!(dictionary.addresses().len(), capacity);
        }
    }

    #[test]
    fn trace_cmp_budget_is_independent_of_coverage_promotion() {
        let mut dictionary = FuzzDictionary::default();
        dictionary.insert_db_values(Vec::new());
        for value in 1..=MAX_COVERAGE_VALUES {
            dictionary.insert_value(B256::from(U256::from(value)));
        }
        dictionary.promote_ephemeral_values();
        dictionary.revert();

        let trace_cmp = B256::from(U256::from(0xdeadbeef_u64));
        let transient = B256::from(U256::from(0xbeef_u64));
        dictionary.insert_value(transient);
        dictionary.insert_persistent_value(trace_cmp);
        dictionary.promote_ephemeral_values();
        dictionary.revert();

        assert_eq!(dictionary.len(), 1 + MAX_COVERAGE_VALUES + 1);
        assert!(dictionary.values().contains(&trace_cmp));
        assert!(!dictionary.values().contains(&transient));
    }

    #[test]
    fn coverage_promotion_stays_worker_local() {
        let seed = EvmFuzzState::test();
        let first = seed.clone().into_invariant();
        let second = seed.clone().into_invariant();
        let readonly = seed.stateless_worker();
        let learned = B256::from(U256::from(0xcafe_u64));
        first.collect_values([learned]);
        first.promote_ephemeral_values();
        first.revert();
        readonly.promote_ephemeral_values();

        assert!(first.with_dictionary(|dictionary| dictionary.values().contains(&learned)));
        assert!(!second.with_dictionary(|dictionary| dictionary.values().contains(&learned)));
        assert!(!readonly.with_dictionary(|dictionary| dictionary.values().contains(&learned)));
    }
}
