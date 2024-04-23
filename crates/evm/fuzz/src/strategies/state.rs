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
    db::{CacheDB, DatabaseRef},
    interpreter::opcode::{self, spec_opcode_gas},
    primitives::SpecId,
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
    pub fn new(dictionary: FuzzDictionary) -> Self {
        Self { inner: Arc::new(RwLock::new(dictionary)) }
    }

    pub fn collect_values(&self, values: impl IntoIterator<Item = [u8; 32]>) {
        let mut dict = self.inner.write();
        for value in values {
            dict.insert_value(value);
        }
    }

    /// Collects call result (if any), state changes from a [StateChangeset] and logs into an
    /// [EvmFuzzState] according to the given [FuzzDictionaryConfig].
    pub fn collect_state_from_call(
        &self,
        function: &Option<Function>,
        abi: &JsonAbi,
        result: &Bytes,
        logs: &[Log],
        state_changeset: &StateChangeset,
        run_depth: u32,
    ) {
        let mut dict = self.inner.write();
        let mut samples = Vec::new();

        match function {
            Some(func) => {
                // Decode result and collect samples to be used in subsequent fuzz runs.
                if !result.is_empty() {
                    if let Ok(decoded_result) = func.abi_decode_output(result, false) {
                        samples.extend(decoded_result);
                    }
                }
            }
            None => {}
        }

        // Decode logs with known events and collect samples from indexed fields and event body.
        for log in logs {
            for event in abi.events() {
                if let Ok(decoded_event) = event.decode_log(log, false) {
                    samples.extend(decoded_event.indexed);
                    samples.extend(decoded_event.body);
                    break;
                }
            }
        }

        // Insert samples collected from current call in fuzz dictionary.
        dict.insert_sample_value(samples, run_depth);

        for (address, account) in state_changeset {
            // Insert basic account information
            dict.insert_value(address.into_word().into());

            if dict.config.include_push_bytes {
                // Insert push bytes
                if let Some(code) = &account.info.code {
                    dict.insert_address(*address);
                    for push_byte in collect_push_bytes(code.bytes()) {
                        dict.insert_value(push_byte);
                    }
                }
            }

            if dict.config.include_storage {
                // Insert storage
                for (slot, value) in &account.storage {
                    let value = value.present_value;
                    dict.insert_value(B256::from(*slot).0);
                    dict.insert_value(B256::from(value).0);
                    // also add the value below and above the storage value to the dictionary.
                    if value != U256::ZERO {
                        let below_value = value - U256::from(1);
                        dict.insert_value(B256::from(below_value).0);
                    }
                    if value != U256::MAX {
                        let above_value = value + U256::from(1);
                        dict.insert_value(B256::from(above_value).0);
                    }
                }
            }
        }
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
    pub fn new(
        initial_values: IndexSet<[u8; 32]>,
        initial_addresses: IndexSet<Address>,
        config: FuzzDictionaryConfig,
    ) -> Self {
        Self {
            state_values: initial_values,
            addresses: initial_addresses,
            config,
            new_values: IndexSet::new(),
            new_addreses: IndexSet::new(),
            sample_values: HashMap::new(),
        }
    }

    pub fn insert_value(&mut self, value: [u8; 32]) {
        if self.state_values.len() < self.config.max_fuzz_dictionary_values &&
            self.state_values.insert(value)
        {
            self.new_values.insert(value);
        }
    }

    /// Insert sample values that are reused across multiple runs.
    /// The number of samples is limited to invariant run depth.
    pub fn insert_sample_value(&mut self, sample_values: Vec<DynSolValue>, limit: u32) {
        for sample in sample_values {
            let sample_type = sample.as_type().unwrap();
            let sample_value = sample.as_word().unwrap().into();

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

    pub fn insert_address(&mut self, address: Address) {
        if self.addresses.len() < self.config.max_fuzz_dictionary_addresses &&
            self.addresses.insert(address)
        {
            self.new_addreses.insert(address);
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

/// Builds the initial [EvmFuzzState] from a database.
pub fn build_initial_state<DB: DatabaseRef>(
    db: &CacheDB<DB>,
    config: FuzzDictionaryConfig,
) -> EvmFuzzState {
    let mut values = IndexSet::new();
    let mut addresses = IndexSet::new();

    // Sort accounts to ensure deterministic dictionary generation from the same setUp state.
    let mut accs = db.accounts.iter().collect::<Vec<_>>();
    accs.sort_by_key(|(address, _)| *address);

    for (address, account) in accs {
        let address: Address = *address;
        // Insert basic account information
        values.insert(address.into_word().into());

        // Insert push bytes
        if config.include_push_bytes {
            if let Some(code) = &account.info.code {
                addresses.insert(address);
                for push_byte in collect_push_bytes(code.bytes()) {
                    values.insert(push_byte);
                }
            }
        }

        if config.include_storage {
            // Insert storage
            for (slot, value) in &account.storage {
                values.insert(B256::from(*slot).0);
                values.insert(B256::from(*value).0);
                // also add the value below and above the storage value to the dictionary.
                if *value != U256::ZERO {
                    let below_value = value - U256::from(1);
                    values.insert(B256::from(below_value).0);
                }
                if *value != U256::MAX {
                    let above_value = value + U256::from(1);
                    values.insert(B256::from(above_value).0);
                }
            }
        }
    }

    // need at least some state data if db is empty otherwise we can't select random data for state
    // fuzzing
    if values.is_empty() {
        // prefill with a random addresses
        values.insert(Address::random().into_word().into());
    }

    EvmFuzzState::new(FuzzDictionary::new(values, addresses, config))
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
