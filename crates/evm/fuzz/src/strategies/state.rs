use crate::invariant::{BasicTxDetails, FuzzRunIdentifiedContracts};
use alloy_dyn_abi::{DynSolType, DynSolValue, EventExt, FunctionExt};
use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{Address, Bytes, Log, B256, U256};
use foundry_config::FuzzDictionaryConfig;
use foundry_evm_core::utils::StateChangeset;
use indexmap::IndexSet;
use parking_lot::{lock_api::RwLockReadGuard, RawRwLock, RwLock};
use revm::{
    db::{CacheDB, DatabaseRef, DbAccount},
    interpreter::opcode,
    primitives::AccountInfo,
};
use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    sync::Arc,
};

type AIndexSet<T> = IndexSet<T, std::hash::BuildHasherDefault<ahash::AHasher>>;

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
}

impl EvmFuzzState {
    pub fn new<DB: DatabaseRef>(db: &CacheDB<DB>, config: FuzzDictionaryConfig) -> Self {
        // Sort accounts to ensure deterministic dictionary generation from the same setUp state.
        let mut accs = db.accounts.iter().collect::<Vec<_>>();
        accs.sort_by_key(|(address, _)| *address);

        // Create fuzz dictionary and insert values from db state.
        let mut dictionary = FuzzDictionary::new(config);
        dictionary.insert_db_values(accs);
        Self { inner: Arc::new(RwLock::new(dictionary)) }
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
        }
        dict.insert_new_state_values(state_changeset);
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
}

// We're using `IndexSet` to have a stable element order when restoring persisted state, as well as
// for performance when iterating over the sets.
#[derive(Default)]
pub struct FuzzDictionary {
    /// Collected state values.
    state_values: AIndexSet<B256>,
    /// Addresses that already had their PUSH bytes collected.
    addresses: AIndexSet<Address>,
    /// Configuration for the dictionary.
    config: FuzzDictionaryConfig,
    /// Number of state values initially collected from db.
    /// Used to revert new collected values at the end of each run.
    db_state_values: usize,
    /// Number of address values initially collected from db.
    /// Used to revert new collected addresses at the end of each run.
    db_addresses: usize,
    /// Sample typed values that are collected from call result and used across invariant runs.
    sample_values: HashMap<DynSolType, AIndexSet<B256>>,

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

impl FuzzDictionary {
    pub fn new(config: FuzzDictionaryConfig) -> Self {
        let mut dictionary = Self { config, ..Default::default() };
        dictionary.prefill();
        dictionary
    }

    /// Insert common values into the dictionary at initialization.
    fn prefill(&mut self) {
        self.insert_value(B256::ZERO);
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
                    self.insert_storage_value(slot, value);
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
        if let Some(function) = function {
            if !function.outputs.is_empty() {
                // Decode result and collect samples to be used in subsequent fuzz runs.
                if let Ok(decoded_result) = function.abi_decode_output(result, false) {
                    self.insert_sample_values(decoded_result, run_depth);
                }
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
                    if let Ok(decoded_event) = event.decode_log(log, false) {
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
    fn insert_new_state_values(&mut self, state_changeset: &StateChangeset) {
        for (address, account) in state_changeset {
            // Insert basic account information.
            self.insert_value(address.into_word());
            // Insert push bytes.
            self.insert_push_bytes_values(address, &account.info);
            // Insert storage values.
            if self.config.include_storage {
                for (slot, value) in &account.storage {
                    self.insert_storage_value(slot, &value.present_value);
                }
            }
        }
    }

    /// Insert values from push bytes into fuzz dictionary.
    /// Values are collected only once for a given address.
    /// If values are newly collected then they are removed at the end of current run.
    fn insert_push_bytes_values(&mut self, address: &Address, account_info: &AccountInfo) {
        if self.config.include_push_bytes && !self.addresses.contains(address) {
            // Insert push bytes
            if let Some(code) = &account_info.code {
                self.insert_address(*address);
                self.collect_push_bytes(code.bytes_slice());
            }
        }
    }

    fn collect_push_bytes(&mut self, code: &[u8]) {
        let mut i = 0;
        let len = code.len().min(PUSH_BYTE_ANALYSIS_LIMIT);
        while i < len {
            let op = code[i];
            if (opcode::PUSH1..=opcode::PUSH32).contains(&op) {
                let push_size = (op - opcode::PUSH1 + 1) as usize;
                let push_start = i + 1;
                let push_end = push_start + push_size;
                // As a precaution, if a fuzz test deploys malformed bytecode (such as using
                // `CREATE2`) this will terminate the loop early.
                if push_start > code.len() || push_end > code.len() {
                    break;
                }

                let push_value = U256::try_from_be_slice(&code[push_start..push_end]).unwrap();
                if push_value != U256::ZERO {
                    // Never add 0 to the dictionary as it's always present.
                    self.insert_value(push_value.into());

                    // Also add the value below and above the push value to the dictionary.
                    self.insert_value((push_value - U256::from(1)).into());

                    if push_value != U256::MAX {
                        self.insert_value((push_value + U256::from(1)).into());
                    }
                }

                i += push_size;
            }
            i += 1;
        }
    }

    /// Insert values from single storage slot and storage value into fuzz dictionary.
    /// If storage values are newly collected then they are removed at the end of current run.
    fn insert_storage_value(&mut self, storage_slot: &U256, storage_value: &U256) {
        self.insert_value(B256::from(*storage_slot));
        self.insert_value(B256::from(*storage_value));
        // also add the value below and above the storage value to the dictionary.
        if *storage_value != U256::ZERO {
            let below_value = storage_value - U256::from(1);
            self.insert_value(B256::from(below_value));
        }
        if *storage_value != U256::MAX {
            let above_value = storage_value + U256::from(1);
            self.insert_value(B256::from(above_value));
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
    /// If value is newly collected then it is removed by index at the end of current run.
    fn insert_value(&mut self, value: B256) {
        if self.state_values.len() < self.config.max_fuzz_dictionary_values {
            let new_value = self.state_values.insert(value);
            let counter = if new_value { &mut self.misses } else { &mut self.hits };
            *counter += 1;
        }
    }

    /// Insert sample values that are reused across multiple runs.
    /// The number of samples is limited to invariant run depth.
    /// If collected samples limit is reached then values are inserted as regular values.
    pub fn insert_sample_values(
        &mut self,
        sample_values: impl IntoIterator<Item = DynSolValue>,
        limit: u32,
    ) {
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

    pub fn values(&self) -> &AIndexSet<B256> {
        &self.state_values
    }

    pub fn len(&self) -> usize {
        self.state_values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.state_values.is_empty()
    }

    #[inline]
    pub fn samples(&self, param_type: &DynSolType) -> Option<&AIndexSet<B256>> {
        self.sample_values.get(param_type)
    }

    #[inline]
    pub fn addresses(&self) -> &AIndexSet<Address> {
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
}
