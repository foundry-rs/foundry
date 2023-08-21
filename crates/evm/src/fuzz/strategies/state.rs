use super::fuzz_param_from_state;
use crate::{
    executor::StateChangeset,
    fuzz::invariant::{ArtifactFilters, FuzzRunIdentifiedContracts},
    utils::{self, b160_to_h160, ru256_to_u256},
};
use bytes::Bytes;
use ethers::{
    abi::Function,
    types::{Address, Log, H256, U256},
};
use foundry_common::contracts::{ContractsByAddress, ContractsByArtifact};
use foundry_config::FuzzDictionaryConfig;
use hashbrown::HashSet;
use parking_lot::RwLock;
use proptest::prelude::{BoxedStrategy, Strategy};
use revm::{
    db::{CacheDB, DatabaseRef},
    interpreter::opcode::{self, spec_opcode_gas},
    primitives::SpecId,
};
use std::{fmt, io::Write, sync::Arc};

/// A set of arbitrary 32 byte data from the VM used to generate values for the strategy.
///
/// Wrapped in a shareable container.
pub type EvmFuzzState = Arc<RwLock<FuzzDictionary>>;

#[derive(Default)]
pub struct FuzzDictionary {
    /// Collected state values.
    state_values: HashSet<[u8; 32]>,
    /// Addresses that already had their PUSH bytes collected.
    addresses: HashSet<Address>,
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
    #[inline]
    pub fn values(&self) -> &HashSet<[u8; 32]> {
        &self.state_values
    }

    #[inline]
    pub fn values_mut(&mut self) -> &mut HashSet<[u8; 32]> {
        &mut self.state_values
    }

    #[inline]
    pub fn addresses(&mut self) -> &HashSet<Address> {
        &self.addresses
    }

    #[inline]
    pub fn addresses_mut(&mut self) -> &mut HashSet<Address> {
        &mut self.addresses
    }
}

/// Given a function and some state, it returns a strategy which generated valid calldata for the
/// given function's input types, based on state taken from the EVM.
pub fn fuzz_calldata_from_state(
    func: Function,
    state: EvmFuzzState,
) -> BoxedStrategy<ethers::types::Bytes> {
    let strats = func
        .inputs
        .iter()
        .map(|input| fuzz_param_from_state(&input.kind, state.clone()))
        .collect::<Vec<_>>();

    strats
        .prop_map(move |tokens| {
            func.encode_input(&tokens)
                .unwrap_or_else(|_| {
                    panic!(
                        "Fuzzer generated invalid tokens for function `{}` with inputs {:?}: {:?}",
                        func.name, func.inputs, tokens
                    )
                })
                .into()
        })
        .no_shrink()
        .boxed()
}

/// Builds the initial [EvmFuzzState] from a database.
pub fn build_initial_state<DB: DatabaseRef>(
    db: &CacheDB<DB>,
    config: &FuzzDictionaryConfig,
) -> EvmFuzzState {
    let mut state = FuzzDictionary::default();

    for (address, account) in db.accounts.iter() {
        let address: Address = (*address).into();
        // Insert basic account information
        state.values_mut().insert(H256::from(address).into());

        // Insert push bytes
        if config.include_push_bytes {
            if let Some(code) = &account.info.code {
                if state.addresses_mut().insert(address) {
                    for push_byte in collect_push_bytes(code.bytes().clone()) {
                        state.values_mut().insert(push_byte);
                    }
                }
            }
        }

        if config.include_storage {
            // Insert storage
            for (slot, value) in &account.storage {
                let slot = (*slot).into();
                let value = (*value).into();
                state.values_mut().insert(utils::u256_to_h256_be(slot).into());
                state.values_mut().insert(utils::u256_to_h256_be(value).into());
                // also add the value below and above the storage value to the dictionary.
                if value != U256::zero() {
                    let below_value = value - U256::one();
                    state.values_mut().insert(utils::u256_to_h256_be(below_value).into());
                }
                if value != U256::max_value() {
                    let above_value = value + U256::one();
                    state.values_mut().insert(utils::u256_to_h256_be(above_value).into());
                }
            }
        }
    }

    // need at least some state data if db is empty otherwise we can't select random data for state
    // fuzzing
    if state.values().is_empty() {
        // prefill with a random addresses
        state.values_mut().insert(H256::from(Address::random()).into());
    }

    Arc::new(RwLock::new(state))
}

/// Collects state changes from a [StateChangeset] and logs into an [EvmFuzzState] according to the
/// given [FuzzDictionaryConfig].
pub fn collect_state_from_call(
    logs: &[Log],
    state_changeset: &StateChangeset,
    state: EvmFuzzState,
    config: &FuzzDictionaryConfig,
) {
    let mut state = state.write();

    for (address, account) in state_changeset {
        // Insert basic account information
        state.values_mut().insert(H256::from(b160_to_h160(*address)).into());

        if config.include_push_bytes && state.addresses.len() < config.max_fuzz_dictionary_addresses
        {
            // Insert push bytes
            if let Some(code) = &account.info.code {
                if state.addresses_mut().insert(b160_to_h160(*address)) {
                    for push_byte in collect_push_bytes(code.bytes().clone()) {
                        state.values_mut().insert(push_byte);
                    }
                }
            }
        }

        if config.include_storage && state.state_values.len() < config.max_fuzz_dictionary_values {
            // Insert storage
            for (slot, value) in &account.storage {
                let slot = (*slot).into();
                let value = ru256_to_u256(value.present_value());
                state.values_mut().insert(utils::u256_to_h256_be(slot).into());
                state.values_mut().insert(utils::u256_to_h256_be(value).into());
                // also add the value below and above the storage value to the dictionary.
                if value != U256::zero() {
                    let below_value = value - U256::one();
                    state.values_mut().insert(utils::u256_to_h256_be(below_value).into());
                }
                if value != U256::max_value() {
                    let above_value = value + U256::one();
                    state.values_mut().insert(utils::u256_to_h256_be(above_value).into());
                }
            }
        } else {
            return
        }

        // Insert log topics and data
        for log in logs {
            log.topics.iter().for_each(|topic| {
                state.values_mut().insert(topic.0);
            });
            log.data.0.chunks(32).for_each(|chunk| {
                let mut buffer: [u8; 32] = [0; 32];
                let _ = (&mut buffer[..])
                    .write(chunk)
                    .expect("log data chunk was larger than 32 bytes");
                state.values_mut().insert(buffer);
            });
        }
    }
}

/// The maximum number of bytes we will look at in bytecodes to find push bytes (24 KiB).
///
/// This is to limit the performance impact of fuzz tests that might deploy arbitrarily sized
/// bytecode (as is the case with Solmate).
const PUSH_BYTE_ANALYSIS_LIMIT: usize = 24 * 1024;

/// Collects all push bytes from the given bytecode.
fn collect_push_bytes(code: Bytes) -> Vec<[u8; 32]> {
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
                return bytes
            }

            let push_value = U256::from_big_endian(&code[push_start..push_end]);
            bytes.push(push_value.into());
            // also add the value below and above the push value to the dictionary.
            if push_value != U256::zero() {
                bytes.push((push_value - U256::one()).into());
            }
            if push_value != U256::max_value() {
                bytes.push((push_value + U256::one()).into());
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
    targeted_contracts: FuzzRunIdentifiedContracts,
    created_contracts: &mut Vec<Address>,
) -> eyre::Result<()> {
    let mut writable_targeted = targeted_contracts.lock();

    for (address, account) in state_changeset {
        if !setup_contracts.contains_key(&b160_to_h160(*address)) {
            if let (true, Some(code)) = (&account.is_touched(), &account.info.code) {
                if !code.is_empty() {
                    if let Some((artifact, (abi, _))) = project_contracts.find_by_code(code.bytes())
                    {
                        if let Some(functions) =
                            artifact_filters.get_targeted_functions(artifact, abi)?
                        {
                            created_contracts.push(b160_to_h160(*address));
                            writable_targeted.insert(
                                b160_to_h160(*address),
                                (artifact.name.clone(), abi.clone(), functions),
                            );
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
