use super::fuzz_param_from_state;
use crate::{executor::StateChangeset, utils};
use bytes::Bytes;
use ethers::{
    abi::Function,
    types::{Address, Log, H256, U256},
};
use proptest::prelude::{BoxedStrategy, Strategy};
use revm::{
    db::{CacheDB, DatabaseRef},
    opcode, spec_opcode_gas, SpecId,
};
use std::{cell::RefCell, collections::HashSet, io::Write, rc::Rc};

/// A set of arbitrary 32 byte data from the VM used to generate values for the strategy.
///
/// Wrapped in a shareable container.
pub type EvmFuzzState = Rc<RefCell<HashSet<[u8; 32]>>>;

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
            tracing::trace!(input = ?tokens);
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
pub fn build_initial_state<DB: DatabaseRef>(db: &CacheDB<DB>) -> EvmFuzzState {
    let mut state: HashSet<[u8; 32]> = HashSet::new();
    for (address, account) in db.accounts.iter() {
        let info = db.basic(*address);

        // Insert basic account information
        state.insert(H256::from(*address).into());
        state.insert(utils::u256_to_h256_le(info.balance).into());
        state.insert(utils::u256_to_h256_le(U256::from(info.nonce)).into());

        // Insert storage
        for (slot, value) in &account.storage {
            state.insert(utils::u256_to_h256_le(*slot).into());
            state.insert(utils::u256_to_h256_le(*value).into());
        }
    }

    // need at least some state data if db is empty otherwise we can't select random data for state
    // fuzzing
    if state.is_empty() {
        // prefill with a random addresses
        state.insert(H256::from(Address::random()).into());
    }

    Rc::new(RefCell::new(state))
}

/// Collects state changes from a [StateChangeset] and logs into an [EvmFuzzState].
pub fn collect_state_from_call(
    logs: &[Log],
    state_changeset: &StateChangeset,
    state: EvmFuzzState,
) {
    let state = &mut *state.borrow_mut();

    for (address, account) in state_changeset {
        // Insert basic account information
        state.insert(H256::from(*address).into());
        state.insert(utils::u256_to_h256_le(account.info.balance).into());
        state.insert(utils::u256_to_h256_le(U256::from(account.info.nonce)).into());

        // Insert storage
        for (slot, value) in &account.storage {
            state.insert(utils::u256_to_h256_le(*slot).into());
            state.insert(utils::u256_to_h256_le(*value).into());
        }

        // Insert push bytes
        if let Some(code) = &account.info.code {
            for push_byte in collect_push_bytes(code.bytes().clone()) {
                state.insert(push_byte);
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

            let mut buffer: [u8; 32] = [0; 32];
            let _ = (&mut buffer[..])
                .write(&code[push_start..push_end])
                .expect("push was larger than 32 bytes");
            bytes.push(buffer);
            i += push_size;
        }
        i += 1;
    }

    bytes
}
