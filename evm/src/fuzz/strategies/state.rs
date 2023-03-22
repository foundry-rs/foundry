use super::fuzz_param_from_state;
use crate::{
    executor::StateChangeset,
    fuzz::invariant::{ArtifactFilters, FuzzRunIdentifiedContracts},
    utils::{self},
};
use bytes::Bytes;
use ethers::{
    abi::Function,
    prelude::rand::{seq::IteratorRandom, thread_rng},
    types::{Address, Log, H256, U256},
};
use foundry_common::contracts::{ContractsByAddress, ContractsByArtifact};
use hashbrown::HashSet;
use parking_lot::RwLock;
use proptest::prelude::{BoxedStrategy, Strategy};
use revm::{
    db::{CacheDB, DatabaseRef},
    opcode, spec_opcode_gas, SpecId,
};
use std::{
    collections::BTreeSet,
    io::Write,
    ops::{Deref, DerefMut},
    sync::Arc,
};

/// A set of arbitrary 32 byte data from the VM used to generate values for the strategy.
///
/// Wrapped in a shareable container.
pub type EvmFuzzState = Arc<RwLock<FuzzDictionary>>;

#[derive(Debug, Default)]
pub struct FuzzDictionary {
    inner: BTreeSet<[u8; 32]>,
    /// Addresses that already had their PUSH bytes collected.
    cache: HashSet<Address>,
}

impl FuzzDictionary {
    /// If the dictionary exceeds these limits it randomly evicts
    pub fn enforce_limit(&mut self, max_addresses: usize, max_values: usize) {
        assert_ne!(max_addresses, 0);
        assert_ne!(max_values, 0);

        if self.inner.len() < max_values && self.cache.len() < max_addresses {
            return
        }
        let mut rng = thread_rng();
        while self.inner.len() > max_values {
            let evict = self.inner.iter().choose(&mut rng).copied().expect("not empty");
            self.inner.remove(&evict);
        }
        while self.cache.len() > max_addresses {
            let evict = self.cache.iter().choose(&mut rng).copied().expect("not empty");
            self.cache.remove(&evict);
        }
    }
}

impl Deref for FuzzDictionary {
    type Target = BTreeSet<[u8; 32]>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for FuzzDictionary {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
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
                        r#"Fuzzer generated invalid tokens {:?} for function `{}` inputs {:?}
This is a bug, please open an issue: https://github.com/foundry-rs/foundry/issues"#,
                        tokens, func.name, func.inputs
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
    include_storage: bool,
    include_push_bytes: bool,
) -> EvmFuzzState {
    let mut state = FuzzDictionary::default();

    for (address, account) in db.accounts.iter() {
        // Insert basic account information
        state.insert(H256::from(*address).into());

        // Insert push bytes
        if include_push_bytes {
            if let Some(code) = &account.info.code {
                if state.cache.insert(*address) {
                    for push_byte in collect_push_bytes(code.bytes().clone()) {
                        state.insert(push_byte);
                    }
                }
            }
        }

        if include_storage {
            // Insert storage
            for (slot, value) in &account.storage {
                state.insert(utils::u256_to_h256_be(*slot).into());
                state.insert(utils::u256_to_h256_be(*value).into());
            }
        }
    }

    // need at least some state data if db is empty otherwise we can't select random data for state
    // fuzzing
    if state.is_empty() {
        // prefill with a random addresses
        state.insert(H256::from(Address::random()).into());
    }

    Arc::new(RwLock::new(state))
}

/// Collects state changes from a [StateChangeset] and logs into an [EvmFuzzState].
pub fn collect_state_from_call(
    logs: &[Log],
    state_changeset: &StateChangeset,
    state: EvmFuzzState,
    include_storage: bool,
    include_push_bytes: bool,
) {
    let mut state = state.write();

    for (address, account) in state_changeset {
        // Insert basic account information
        state.insert(H256::from(*address).into());

        if include_storage {
            // Insert storage
            for (slot, value) in &account.storage {
                state.insert(utils::u256_to_h256_be(*slot).into());
                state.insert(utils::u256_to_h256_be(value.present_value()).into());
            }
        }

        if include_push_bytes {
            // Insert push bytes
            if let Some(code) = &account.info.code {
                if state.cache.insert(*address) {
                    for push_byte in collect_push_bytes(code.bytes().clone()) {
                        state.insert(push_byte);
                    }
                }
            }
        }

        // Insert log topics and data
        for log in logs {
            log.topics.iter().for_each(|topic| {
                state.insert(topic.0);
            });
            log.data.0.chunks(32).for_each(|chunk| {
                let mut buffer: [u8; 32] = [0; 32];
                let _ = (&mut buffer[..])
                    .write(chunk)
                    .expect("log data chunk was larger than 32 bytes");
                state.insert(buffer);
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

            bytes.push(U256::from_big_endian(&code[push_start..push_end]).into());

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
        if !setup_contracts.contains_key(address) {
            if let (true, Some(code)) = (&account.is_touched, &account.info.code) {
                if !code.is_empty() {
                    if let Some((artifact, (abi, _))) = project_contracts.find_by_code(code.bytes())
                    {
                        if let Some(functions) =
                            artifact_filters.get_targeted_functions(artifact, abi)?
                        {
                            created_contracts.push(*address);
                            writable_targeted
                                .insert(*address, (artifact.name.clone(), abi.clone(), functions));
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
