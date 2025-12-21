use crate::{
    BasicTxDetails, invariant::FuzzRunIdentifiedContracts, strategies::literals::LiteralsDictionary,
};
use alloy_dyn_abi::{DynSolType, DynSolValue, EventExt, FunctionExt};
use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{
    Address, B256, Bytes, Log, U256,
    map::{AddressIndexSet, AddressMap, B256IndexSet, HashMap, IndexSet},
};
use foundry_common::{
    ignore_metadata_hash, mapping_slots::MappingSlots, slot_identifier::SlotIdentifier,
};
use foundry_compilers::artifacts::StorageLayout;
use foundry_config::FuzzDictionaryConfig;
use foundry_evm_core::{bytecode::InstIter, utils::StateChangeset};
use parking_lot::{RawRwLock, RwLock, lock_api::RwLockReadGuard};
use revm::{
    database::{CacheDB, DatabaseRef, DbAccount},
    state::AccountInfo,
};
use std::{collections::BTreeMap, fmt, sync::Arc};

/// The maximum number of bytes we will look at in bytecodes to find push bytes (24 KiB).
///
/// This is to limit the performance impact of fuzz tests that might deploy arbitrarily sized
/// bytecode (as is the case with Solmate).
const PUSH_BYTE_ANALYSIS_LIMIT: usize = 24 * 1024;

/// A set of arbitrary 32 byte data from the VM used to generate values for the strategy.
///
/// Wrapped in a shareable container.
#[derive(Clone, Debug)]
pub struct EvmFuzzState {
    inner: Arc<RwLock<FuzzDictionary>>,
    /// Addresses of external libraries deployed in test setup, excluded from fuzz test inputs.
    pub deployed_libs: Vec<Address>,
    /// Records mapping accesses. Used to identify storage slots belonging to mappings and sampling
    /// the values in the [`FuzzDictionary`].
    ///
    /// Only needed when [`StorageLayout`] is available.
    pub(crate) mapping_slots: Option<AddressMap<MappingSlots>>,
}

impl EvmFuzzState {
    #[cfg(test)]
    pub(crate) fn test() -> Self {
        Self::new(
            &[],
            &CacheDB::<revm::database::EmptyDB>::default(),
            FuzzDictionaryConfig::default(),
            None,
        )
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

        Self {
            inner: Arc::new(RwLock::new(dictionary)),
            deployed_libs: deployed_libs.to_vec(),
            mapping_slots: None,
        }
    }

    pub fn with_mapping_slots(mut self, mapping_slots: AddressMap<MappingSlots>) -> Self {
        self.mapping_slots = Some(mapping_slots);
        self
    }

    pub fn collect_values(&self, values: impl IntoIterator<Item = B256>) {
        let mut dict = self.inner.write();
        for value in values {
            dict.insert_value(value);
        }
    }

    /// Collects state changes from a [StateChangeset] and logs into an [EvmFuzzState] according to
    /// the given [FuzzDictionaryConfig].
    pub fn collect_values_from_call(
        &self,
        fuzzed_contracts: &FuzzRunIdentifiedContracts,
        tx: &BasicTxDetails,
        result: &Bytes,
        logs: &[Log],
        state_changeset: &StateChangeset,
        run_depth: u32,
    ) {
        let mut dict = self.inner.write();
        {
            let targets = fuzzed_contracts.targets.lock();
            let (target_abi, target_function) = targets.fuzzed_artifacts(tx);
            dict.insert_logs_values(target_abi, logs, run_depth);
            dict.insert_result_values(target_function, result, run_depth);
            // Get storage layouts for contracts in the state changeset
            let storage_layouts = targets.get_storage_layouts();
            dict.insert_new_state_values(
                state_changeset,
                &storage_layouts,
                self.mapping_slots.as_ref(),
            );
        }
    }

    /// Removes all newly added entries from the dictionary.
    ///
    /// Should be called between fuzz/invariant runs to avoid accumulating data derived from fuzz
    /// inputs.
    pub fn revert(&self) {
        self.inner.write().revert();
    }

    pub fn dictionary_read(&self) -> RwLockReadGuard<'_, RawRwLock, FuzzDictionary> {
        self.inner.read()
    }

    /// Logs stats about the current state.
    pub fn log_stats(&self) {
        self.inner.read().log_stats();
    }

    /// Test-only helper to seed the dictionary with literal values.
    #[cfg(test)]
    pub(crate) fn seed_literals(&self, map: super::LiteralMaps) {
        self.inner.write().seed_literals(map);
    }
}

// We're using `IndexSet` to have a stable element order when restoring persisted state, as well as
// for performance when iterating over the sets.
pub struct FuzzDictionary {
    /// Collected state values.
    state_values: B256IndexSet,
    /// Addresses that already had their PUSH bytes collected.
    addresses: AddressIndexSet,
    /// Configuration for the dictionary.
    config: FuzzDictionaryConfig,
    /// Number of state values initially collected from db.
    /// Used to revert new collected values at the end of each run.
    db_state_values: usize,
    /// Number of address values initially collected from db.
    /// Used to revert new collected addresses at the end of each run.
    db_addresses: usize,
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

    misses: usize,
    hits: usize,
}

impl fmt::Debug for FuzzDictionary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FuzzDictionary")
            .field("state_values", &self.state_values.len())
            .field("addresses", &self.addresses)
            .finish()
    }
}

impl Default for FuzzDictionary {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

impl FuzzDictionary {
    pub fn new(config: FuzzDictionaryConfig) -> Self {
        let mut dictionary = Self {
            config,
            samples_seeded: false,

            state_values: Default::default(),
            addresses: Default::default(),
            db_state_values: Default::default(),
            db_addresses: Default::default(),
            sample_values: Default::default(),
            literal_values: Default::default(),
            misses: Default::default(),
            hits: Default::default(),
        };
        dictionary.prefill();
        dictionary
    }

    /// Insert common values into the dictionary at initialization.
    fn prefill(&mut self) {
        self.insert_value(B256::ZERO);
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
            // Insert basic account information
            self.insert_value(address.into_word());
            // Insert push bytes
            self.insert_push_bytes_values(address, &account.info);
            // Insert storage values.
            if self.config.include_storage {
                // Sort storage values before inserting to ensure deterministic dictionary.
                let values = account.storage.iter().collect::<BTreeMap<_, _>>();
                for (slot, value) in values {
                    self.insert_storage_value(slot, value, None, None);
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
    fn insert_logs_values(&mut self, abi: Option<&JsonAbi>, logs: &[Log], run_depth: u32) {
        let mut samples = Vec::new();
        // Decode logs with known events and collect samples from indexed fields and event body.
        for log in logs {
            let mut log_decoded = false;
            // Try to decode log with events from contract abi.
            if let Some(abi) = abi {
                for event in abi.events() {
                    if let Ok(decoded_event) = event.decode_log(log) {
                        samples.extend(decoded_event.indexed);
                        samples.extend(decoded_event.body);
                        log_decoded = true;
                        break;
                    }
                }
            }

            // If we weren't able to decode event then we insert raw data in fuzz dictionary.
            if !log_decoded {
                for &topic in log.topics() {
                    self.insert_value(topic);
                }
                let chunks = log.data.data.chunks_exact(32);
                let rem = chunks.remainder();
                for chunk in chunks {
                    self.insert_value(chunk.try_into().unwrap());
                }
                if !rem.is_empty() {
                    self.insert_value(B256::right_padding_from(rem));
                }
            }
        }

        // Insert samples collected from current call in fuzz dictionary.
        self.insert_sample_values(samples, run_depth);
    }

    /// Insert values from call state changeset into fuzz dictionary.
    /// These values are removed at the end of current run.
    fn insert_new_state_values(
        &mut self,
        state_changeset: &StateChangeset,
        storage_layouts: &HashMap<Address, Arc<StorageLayout>>,
        mapping_slots: Option<&AddressMap<MappingSlots>>,
    ) {
        for (address, account) in state_changeset {
            // Insert basic account information.
            self.insert_value(address.into_word());
            // Insert push bytes.
            self.insert_push_bytes_values(address, &account.info);
            // Insert storage values.
            if self.config.include_storage {
                let storage_layout = storage_layouts.get(address).cloned();
                trace!(
                    "{address:?} has mapping_slots {}",
                    mapping_slots.is_some_and(|m| m.contains_key(address))
                );
                let mapping_slots = mapping_slots.and_then(|m| m.get(address));
                for (slot, value) in &account.storage {
                    self.insert_storage_value(
                        slot,
                        &value.present_value,
                        storage_layout.as_deref(),
                        mapping_slots,
                    );
                }
            }
        }
    }

    /// Insert values from push bytes into fuzz dictionary.
    /// Values are collected only once for a given address.
    /// If values are newly collected then they are removed at the end of current run.
    fn insert_push_bytes_values(&mut self, address: &Address, account_info: &AccountInfo) {
        if self.config.include_push_bytes
            && !self.addresses.contains(address)
            && let Some(code) = &account_info.code
        {
            self.insert_address(*address);
            if !self.values_full() {
                self.collect_push_bytes(ignore_metadata_hash(code.original_byte_slice()));
            }
        }
    }

    fn collect_push_bytes(&mut self, code: &[u8]) {
        let len = code.len().min(PUSH_BYTE_ANALYSIS_LIMIT);
        let code = &code[..len];
        for inst in InstIter::new(code) {
            // Don't add 0 to the dictionary as it's already present.
            if !inst.immediate.is_empty()
                && let Some(push_value) = U256::try_from_be_slice(inst.immediate)
                && push_value != U256::ZERO
            {
                self.insert_value_u256(push_value);
            }
        }
    }

    /// Insert values from single storage slot and storage value into fuzz dictionary.
    /// Uses [`SlotIdentifier`] to identify storage slots types.
    fn insert_storage_value(
        &mut self,
        slot: &U256,
        value: &U256,
        layout: Option<&StorageLayout>,
        mapping_slots: Option<&MappingSlots>,
    ) {
        let slot = B256::from(*slot);
        let value = B256::from(*value);

        // Always insert the slot itself
        self.insert_value(slot);

        // If we have a storage layout, use SlotIdentifier for better type identification
        if let Some(slot_identifier) =
            layout.map(|l| SlotIdentifier::new(l.clone().into()))
            // Identify Slot Type
            && let Some(slot_info) = slot_identifier.identify(&slot, mapping_slots) && slot_info.decode(value).is_some()
        {
            trace!(?slot_info, "inserting typed storage value");
            if !self.samples_seeded {
                self.seed_samples();
            }
            self.sample_values.entry(slot_info.slot_type.dyn_sol_type).or_default().insert(value);
        } else {
            self.insert_value_u256(value.into());
        }
    }

    /// Insert address into fuzz dictionary.
    /// If address is newly collected then it is removed by index at the end of current run.
    fn insert_address(&mut self, address: Address) {
        if self.addresses.len() < self.config.max_fuzz_dictionary_addresses {
            self.addresses.insert(address);
        }
    }

    /// Insert raw value into fuzz dictionary.
    ///
    /// If value is newly collected then it is removed by index at the end of current run.
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

    fn insert_value_u256(&mut self, value: U256) -> bool {
        // Also add the value below and above the push value to the dictionary.
        let one = U256::from(1);
        self.insert_value(value.into())
            | self.insert_value((value.wrapping_sub(one)).into())
            | self.insert_value((value.wrapping_add(one)).into())
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

    pub fn values(&self) -> &B256IndexSet {
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
    pub fn addresses(&self) -> &AddressIndexSet {
        &self.addresses
    }

    /// Revert values and addresses collected during the run by truncating to initial db len.
    pub fn revert(&mut self) {
        self.state_values.truncate(self.db_state_values);
        self.addresses.truncate(self.db_addresses);
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

    #[cfg(test)]
    /// Test-only helper to seed the dictionary with literal values.
    pub(crate) fn seed_literals(&mut self, map: super::LiteralMaps) {
        self.literal_values.set(map);
    }
}
