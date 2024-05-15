use crate::invariant::{ArtifactFilters, FuzzRunIdentifiedContracts};
use alloy_dyn_abi::{DynSolType, DynSolValue, EventExt, FunctionExt};
use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{Address, Bytes, Log, B256, U256};
use foundry_common::contracts::{ContractsByAddress, ContractsByArtifact};
use foundry_config::FuzzDictionaryConfig;
use foundry_evm_core::utils::StateChangeset;
use indexmap::IndexSet;
use parking_lot::{lock_api::RwLockReadGuard, RawRwLock, RwLock};
use revm::{
    db::{CacheDB, DatabaseRef, DbAccount},
    interpreter::opcode::{self, spec_opcode_gas},
    primitives::{AccountInfo, SpecId},
};
use std::{collections::HashMap, fmt, sync::Arc};

/// A set of arbitrary 32 byte data from the VM used to generate values for the strategy.
///
/// Wrapped in a shareable container.
#[derive(Clone, Debug)]
pub struct EvmFuzzState {
    inner: Arc<RwLock<FuzzDictionary>>,
}

impl EvmFuzzState {
    pub fn new<DB: DatabaseRef>(db: &CacheDB<DB>, config: FuzzDictionaryConfig) -> EvmFuzzState {
        // Sort accounts to ensure deterministic dictionary generation from the same setUp state.
        let mut accs = db.accounts.iter().collect::<Vec<_>>();
        accs.sort_by_key(|(address, _)| *address);

        // Create fuzz dictionary and insert values from db state.
        let mut dictionary = FuzzDictionary::new(config);
        dictionary.insert_db_values(accs);
        Self { inner: Arc::new(RwLock::new(dictionary)) }
    }

    pub fn collect_values(&self, values: impl IntoIterator<Item = [u8; 32]>) {
        let mut dict = self.inner.write();
        for value in values {
            dict.insert_value(value, true);
        }
    }

    /// Collects state changes from a [StateChangeset] and logs into an [EvmFuzzState] according to
    /// the given [FuzzDictionaryConfig].
    pub fn collect_values_from_call(
        &self,
        target_abi: Option<&JsonAbi>,
        target_function: Option<&Function>,
        result: &Bytes,
        logs: &[Log],
        state_changeset: &StateChangeset,
        run_depth: u32,
    ) {
        let mut dict = self.inner.write();
        dict.insert_result_values(target_function, result, run_depth);
        dict.insert_logs_values(target_abi, logs, run_depth);
        dict.insert_state_values(state_changeset);
    }

    /// Removes all newly added entries from the dictionary.
    ///
    /// Should be called between fuzz/invariant runs to avoid accumumlating data derived from fuzz
    /// inputs.
    pub fn revert(&self) {
        self.inner.write().revert();
    }

    pub fn dictionary_read(&self) -> RwLockReadGuard<'_, RawRwLock, FuzzDictionary> {
        self.inner.read()
    }
}

// We're using `IndexSet` to have a stable element order when restoring persisted state, as well as
// for performance when iterating over the sets.
#[derive(Default)]
pub struct FuzzDictionary {
    /// Collected state values.
    state_values: IndexSet<[u8; 32]>,
    /// Addresses that already had their PUSH bytes collected.
    addresses: IndexSet<Address>,
    /// Configuration for the dictionary.
    config: FuzzDictionaryConfig,
    /// New keys added to the dictionary since container initialization.
    new_values: IndexSet<[u8; 32]>,
    /// New addresses added to the dictionary since container initialization.
    new_addreses: IndexSet<Address>,
    /// Sample typed values that are collected from call result and used across invariant runs.
    sample_values: HashMap<DynSolType, IndexSet<[u8; 32]>>,
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
        Self { config, ..Default::default() }
    }

    /// Insert values from initial db state into fuzz dictionary.
    /// These values are persisted across invariant runs.
    fn insert_db_values(&mut self, db_state: Vec<(&Address, &DbAccount)>) {
        for (address, account) in db_state {
            // Insert basic account information
            self.insert_value(address.into_word().into(), false);
            // Insert push bytes
            self.insert_push_bytes_values(address, &account.info, false);
            // Insert storage values.
            if self.config.include_storage {
                for (slot, value) in &account.storage {
                    self.insert_storage_value(slot, value, false);
                }
            }
        }

        // need at least some state data if db is empty otherwise we can't select random data for
        // state fuzzing
        if self.values().is_empty() {
            // prefill with a random addresses
            self.insert_value(Address::random().into_word().into(), false);
        }
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
                for topic in log.topics() {
                    self.insert_value(topic.0, true);
                }
                let chunks = log.data.data.chunks_exact(32);
                let rem = chunks.remainder();
                for chunk in chunks {
                    self.insert_value(chunk.try_into().unwrap(), true);
                }
                if !rem.is_empty() {
                    self.insert_value(B256::right_padding_from(rem).0, true);
                }
            }
        }

        // Insert samples collected from current call in fuzz dictionary.
        self.insert_sample_values(samples, run_depth);
    }

    /// Insert values from call state changeset into fuzz dictionary.
    /// These values are removed at the end of current run.
    fn insert_state_values(&mut self, state_changeset: &StateChangeset) {
        for (address, account) in state_changeset {
            // Insert basic account information.
            self.insert_value(address.into_word().into(), true);
            // Insert push bytes.
            self.insert_push_bytes_values(address, &account.info, true);
            // Insert storage values.
            if self.config.include_storage {
                for (slot, value) in &account.storage {
                    self.insert_storage_value(slot, &value.present_value, true);
                }
            }
        }
    }

    /// Insert values from push bytes into fuzz dictionary.
    /// If values are newly collected then they are removed at the end of current run.
    fn insert_push_bytes_values(
        &mut self,
        address: &Address,
        account_info: &AccountInfo,
        collected: bool,
    ) {
        if self.config.include_push_bytes {
            // Insert push bytes
            if let Some(code) = account_info.code.clone() {
                self.insert_address(*address, collected);
                for push_byte in collect_push_bytes(code.bytes()) {
                    self.insert_value(push_byte, collected);
                }
            }
        }
    }

    /// Insert values from single storage slot and storage value into fuzz dictionary.
    /// If storage values are newly collected then they are removed at the end of current run.
    fn insert_storage_value(&mut self, storage_slot: &U256, storage_value: &U256, collected: bool) {
        self.insert_value(B256::from(*storage_slot).0, collected);
        self.insert_value(B256::from(*storage_value).0, collected);
        // also add the value below and above the storage value to the dictionary.
        if *storage_value != U256::ZERO {
            let below_value = storage_value - U256::from(1);
            self.insert_value(B256::from(below_value).0, collected);
        }
        if *storage_value != U256::MAX {
            let above_value = storage_value + U256::from(1);
            self.insert_value(B256::from(above_value).0, collected);
        }
    }

    /// Insert address into fuzz dictionary.
    /// If address is newly collected then it is removed at the end of current run.
    fn insert_address(&mut self, address: Address, collected: bool) {
        if self.addresses.len() < self.config.max_fuzz_dictionary_addresses &&
            self.addresses.insert(address) &&
            collected
        {
            self.new_addreses.insert(address);
        }
    }

    /// Insert raw value into fuzz dictionary.
    /// If value is newly collected then it is removed at the end of current run.
    fn insert_value(&mut self, value: [u8; 32], collected: bool) {
        if self.state_values.len() < self.config.max_fuzz_dictionary_values &&
            self.state_values.insert(value) &&
            collected
        {
            self.new_values.insert(value);
        }
    }

    /// Insert sample values that are reused across multiple runs.
    /// The number of samples is limited to invariant run depth.
    /// If collected samples limit is reached then values are inserted as regular values.
    pub fn insert_sample_values(&mut self, sample_values: Vec<DynSolValue>, limit: u32) {
        for sample in sample_values {
            let sample_type = sample.as_type().unwrap();
            let sample_value = sample.as_word().unwrap().into();

            if let Some(values) = self.sample_values.get_mut(&sample_type) {
                if values.len() < limit as usize {
                    values.insert(sample_value);
                } else {
                    // Insert as state value (will be removed at the end of the run).
                    self.insert_value(sample_value, true);
                }
            } else {
                self.sample_values.entry(sample_type).or_default().insert(sample_value);
            }
        }
    }

    #[inline]
    pub fn values(&self) -> &IndexSet<[u8; 32]> {
        &self.state_values
    }

    #[inline]
    pub fn samples(&self, param_type: DynSolType) -> Option<&IndexSet<[u8; 32]>> {
        self.sample_values.get(&param_type)
    }

    #[inline]
    pub fn addresses(&self) -> &IndexSet<Address> {
        &self.addresses
    }

    pub fn revert(&mut self) {
        // Revert new values collected during the run.
        for key in self.new_values.iter() {
            self.state_values.swap_remove(key);
        }
        for address in self.new_addreses.iter() {
            self.addresses.swap_remove(address);
        }

        self.new_values.clear();
        self.new_addreses.clear();
    }
}

/// The maximum number of bytes we will look at in bytecodes to find push bytes (24 KiB).
///
/// This is to limit the performance impact of fuzz tests that might deploy arbitrarily sized
/// bytecode (as is the case with Solmate).
const PUSH_BYTE_ANALYSIS_LIMIT: usize = 24 * 1024;

/// Collects all push bytes from the given bytecode.
fn collect_push_bytes(code: &[u8]) -> Vec<[u8; 32]> {
    let mut bytes: Vec<[u8; 32]> = Vec::new();
    // We use [SpecId::LATEST] since we do not really care what spec it is - we are not interested
    // in gas costs.
    let opcode_infos = spec_opcode_gas(SpecId::LATEST);
    let mut i = 0;
    while i < code.len().min(PUSH_BYTE_ANALYSIS_LIMIT) {
        let op = code[i];
        if opcode_infos[op as usize].is_push() {
            let push_size = (op - opcode::PUSH1 + 1) as usize;
            let push_start = i + 1;
            let push_end = push_start + push_size;
            // As a precaution, if a fuzz test deploys malformed bytecode (such as using `CREATE2`)
            // this will terminate the loop early.
            if push_start > code.len() || push_end > code.len() {
                return bytes;
            }

            let push_value = U256::try_from_be_slice(&code[push_start..push_end]).unwrap();
            bytes.push(push_value.to_be_bytes());
            // also add the value below and above the push value to the dictionary.
            if push_value != U256::ZERO {
                bytes.push((push_value - U256::from(1)).to_be_bytes());
            }
            if push_value != U256::MAX {
                bytes.push((push_value + U256::from(1)).to_be_bytes());
            }

            i += push_size;
        }
        i += 1;
    }
    bytes
}

/// Collects all created contracts from a StateChangeset which haven't been discovered yet. Stores
/// them at `targeted_contracts` and `created_contracts`.
pub fn collect_created_contracts(
    state_changeset: &StateChangeset,
    project_contracts: &ContractsByArtifact,
    setup_contracts: &ContractsByAddress,
    artifact_filters: &ArtifactFilters,
    targeted_contracts: &FuzzRunIdentifiedContracts,
    created_contracts: &mut Vec<Address>,
) -> eyre::Result<()> {
    let mut writable_targeted = targeted_contracts.targets.lock();
    for (address, account) in state_changeset {
        if !setup_contracts.contains_key(address) {
            if let (true, Some(code)) = (&account.is_touched(), &account.info.code) {
                if !code.is_empty() {
                    if let Some((artifact, contract)) =
                        project_contracts.find_by_deployed_code(&code.original_bytes())
                    {
                        if let Some(functions) =
                            artifact_filters.get_targeted_functions(artifact, &contract.abi)?
                        {
                            created_contracts.push(*address);
                            writable_targeted.insert(
                                *address,
                                (artifact.name.clone(), contract.abi.clone(), functions),
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
