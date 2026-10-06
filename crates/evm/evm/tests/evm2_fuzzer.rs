//! Parity between the REVM and evm2 hooks of the fuzzer inspector.
//!
//! Each case runs the same handler on a bare inspected REVM EVM and on evm2 with only the
//! [`Fuzzer`] attached, then compares the call output and the fuzzer's observations.

use alloy_primitives::{
    Address, B256, Bytes, TxKind, U256,
    map::{AddressMap, AddressSet},
};
use evm2::{SpecId, evm::AccountInfo};
use foundry_evm::ethereum::EthereumExecutor;
use foundry_evm_core::{
    ethereum::{EthereumEnv, LocalState},
    opts::EvmOpts,
};
use foundry_evm_fuzz::{
    BasicTxDetails, CallDetails, Fuzzer, ObservedCall, invariant::RandomCallGenerator,
};
use parking_lot::RwLock;
use proptest::{strategy::Just, test_runner::TestRunner};
use revm::{
    Context, InspectEvm, MainBuilder, MainContext,
    context::TxEnv,
    database::{CacheDB, EmptyDB},
    state::Bytecode,
};
use std::sync::Arc;

const SENDER: Address = Address::repeat_byte(0xaa);
const HANDLER: Address = Address::repeat_byte(0xbb);
const RECIPIENT: Address = Address::repeat_byte(0xcc);
const TEST: Address = Address::repeat_byte(0xdd);

/// Without calldata: sends 5 wei to `RECIPIENT`, writes `mapping[0x11] = 7` for a mapping at slot
/// 2, and returns `(sload(0), balance(RECIPIENT))`. With calldata: stores 1 in slot 0.
fn handler_code() -> Bytes {
    let mut code = vec![0x36, 0x60, 0x54, 0x57, 0x5f, 0x5f, 0x5f, 0x5f, 0x60, 0x05, 0x73];
    code.extend_from_slice(RECIPIENT.as_slice());
    code.extend([
        0x5a, 0xf1, 0x50, 0x60, 0x11, 0x5f, 0x52, 0x60, 0x02, 0x60, 0x20, 0x52, 0x60, 0x40, 0x5f,
        0x20, 0x60, 0x07, 0x90, 0x55, 0x5f, 0x54, 0x5f, 0x52, 0x73,
    ]);
    code.extend_from_slice(RECIPIENT.as_slice());
    code.extend([
        0x31, 0x60, 0x20, 0x52, 0x60, 0x40, 0x5f, 0xf3, 0x5b, 0x60, 0x01, 0x5f, 0x55, 0x00,
    ]);
    assert_eq!(code[0x54], 0x5b);
    code.into()
}

/// Returns a fuzzer recording mapping slots and sub-calls, optionally replaying one reentrant
/// callback from `RECIPIENT` into the handler.
fn fuzzer(reenter: bool) -> Fuzzer {
    let mut fuzzer = Fuzzer::new(64, Some(AddressMap::default())).with_call_recording(true);
    if reenter {
        let callback =
            CallDetails { target: HANDLER, calldata: Bytes::from_static(&[1]), value: None };
        let mut generator = RandomCallGenerator::new(
            TEST,
            AddressSet::from_iter([HANDLER]),
            TestRunner::default(),
            Just(callback.clone()),
            Arc::new(RwLock::new(Address::ZERO)),
        );
        generator.replay = true;
        generator.last_sequence.write().push(Some(BasicTxDetails {
            warp: None,
            roll: None,
            sender: RECIPIENT,
            call_details: callback,
        }));
        fuzzer.call_generator = Some(generator);
    }
    fuzzer
}

fn run_revm(fuzzer: Fuzzer) -> (Bytes, Fuzzer) {
    let mut db = CacheDB::new(EmptyDB::default());
    let code = Bytecode::new_raw(handler_code());
    db.insert_account_info(
        HANDLER,
        revm::state::AccountInfo {
            balance: U256::from(100),
            code_hash: code.hash_slow(),
            code: Some(code),
            ..Default::default()
        },
    );
    let mut evm = Context::mainnet().with_db(db).build_mainnet_with_inspector(fuzzer);
    let tx = TxEnv::builder()
        .caller(SENDER)
        .kind(TxKind::Call(HANDLER))
        .gas_limit(1_000_000)
        .build()
        .unwrap();
    let result = evm.inspect_one_tx(tx).unwrap();
    assert!(result.is_success(), "{result:?}");
    (result.output().unwrap().clone(), evm.inspector)
}

fn run_evm2(fuzzer: Fuzzer) -> (Bytes, Fuzzer) {
    let mut handler =
        AccountInfo::default().with_code(evm2::bytecode::Bytecode::new_legacy(handler_code()));
    handler.balance = U256::from(100);
    let mut state = LocalState::default();
    state.database_mut().insert_account_info(&HANDLER, handler);
    let mut opts = EvmOpts::default();
    opts.env.gas_limit = 30_000_000u64.into();
    opts.memory_limit = 1 << 20;
    let executor =
        EthereumExecutor::with_inspector(EthereumEnv::local(SpecId::CANCUN, &opts), state, fuzzer);
    let (result, fuzzer) = executor.inspect_raw(SENDER, HANDLER, Bytes::new(), U256::ZERO).unwrap();
    assert!(result.status, "{:?}", result.stop);
    (result.output, fuzzer)
}

/// Observations compared across engines.
#[derive(Debug, PartialEq)]
struct Observations {
    output: (U256, U256),
    collected_values: Vec<B256>,
    mapping_hashes: Vec<(B256, (B256, B256))>,
    mapping_keys: Vec<(B256, B256)>,
    observed_calls: Vec<ObservedCall>,
    override_depth: Option<usize>,
    replays_left: Option<usize>,
}

fn observations((output, mut fuzzer): (Bytes, Fuzzer)) -> Observations {
    let slots = fuzzer.mapping_slots.take().unwrap().remove(&HANDLER).unwrap_or_default();
    let mut mapping_hashes = slots.seen_sha3.into_iter().collect::<Vec<_>>();
    mapping_hashes.sort();
    let mut mapping_keys = slots.keys.into_iter().collect::<Vec<_>>();
    mapping_keys.sort();
    Observations {
        output: (U256::from_be_slice(&output[..32]), U256::from_be_slice(&output[32..])),
        collected_values: fuzzer.collected_values.clone(),
        mapping_hashes,
        mapping_keys,
        observed_calls: fuzzer.take_observed_calls(),
        override_depth: fuzzer.call_generator.as_ref().map(|g| g.override_depth),
        replays_left: fuzzer.call_generator.as_ref().map(|g| g.last_sequence.read().len()),
    }
}

#[test]
fn evm2_fuzzer_observations_match_revm() {
    for reenter in [false, true] {
        let revm = observations(run_revm(fuzzer(reenter)));
        let evm2 = observations(run_evm2(fuzzer(reenter)));

        // The value transfer reaches the recipient either way; only the reentrant callback
        // writes slot 0.
        assert_eq!(revm.output, (U256::from(reenter), U256::from(5)), "reenter: {reenter}");
        assert!(!revm.collected_values.is_empty());
        assert_eq!(revm.mapping_keys.len(), 1);
        assert_eq!(revm.observed_calls.len(), 1);
        assert_eq!(evm2, revm, "reenter: {reenter}");
    }
}
