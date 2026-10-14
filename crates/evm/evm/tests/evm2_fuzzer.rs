//! Native fuzzer feedback and reentrant override execution.

use alloy_consensus::{TxLegacy, transaction::Recovered};
use alloy_primitives::{
    Address, B256, Bytes, TxKind, U256,
    map::{AddressMap, AddressSet},
};
use evm2::{
    EvmFeatures, ExecutionConfig, SpecId, Version,
    env::BlockEnvExt,
    ethereum::TxEnvelope,
    evm::{AccountInfo, InMemoryDB},
};
use foundry_evm::ethereum::Executor;
use foundry_evm_fuzz::{
    BasicTxDetails, CallDetails, Fuzzer, ObservedCall, invariant::RandomCallGenerator,
};
use parking_lot::RwLock;
use proptest::{strategy::Just, test_runner::TestRunner};
use std::sync::Arc;

const SENDER: Address = Address::repeat_byte(0xaa);
const HANDLER: Address = Address::repeat_byte(0xbb);
const RECIPIENT: Address = Address::repeat_byte(0xcc);
const TEST: Address = Address::repeat_byte(0xdd);

/// Without calldata: sends 5 wei to `RECIPIENT`, writes `mapping[0x11] = 7` for a mapping at slot
/// 2, hashes 32 bytes that are not a mapping key, and returns `(sload(0), balance(RECIPIENT))`.
/// With calldata: stores 1 in slot 0.
fn handler_code() -> Bytes {
    let mut code = vec![0x36, 0x60, 0x59, 0x57, 0x5f, 0x5f, 0x5f, 0x5f, 0x60, 0x05, 0x73];
    code.extend_from_slice(RECIPIENT.as_slice());
    code.extend([
        0x5a, 0xf1, 0x50, 0x60, 0x11, 0x5f, 0x52, 0x60, 0x02, 0x60, 0x20, 0x52, 0x60, 0x40, 0x5f,
        0x20, 0x60, 0x07, 0x90, 0x55, 0x60, 0x20, 0x5f, 0x20, 0x50, 0x5f, 0x54, 0x5f, 0x52, 0x73,
    ]);
    code.extend_from_slice(RECIPIENT.as_slice());
    code.extend([
        0x31, 0x60, 0x20, 0x52, 0x60, 0x40, 0x5f, 0xf3, 0x5b, 0x60, 0x01, 0x5f, 0x55, 0x00,
    ]);
    assert_eq!(code[0x59], 0x5b);
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

fn run_evm2(fuzzer: Fuzzer) -> (Bytes, Fuzzer) {
    let spec = SpecId::CANCUN;
    let mut version = Version::new(spec);
    version.features.remove(
        EvmFeatures::NONCE_CHECK | EvmFeatures::EIP3607 | EvmFeatures::BLOCK_GAS_LIMIT_CHECK,
    );
    let mut handler =
        AccountInfo::default().with_code(evm2::bytecode::Bytecode::new_legacy(handler_code()));
    handler.balance = U256::from(100);
    let mut db = InMemoryDB::default();
    db.insert_account_info(&HANDLER, handler);
    let executor = Executor::new(
        db,
        spec,
        ExecutionConfig::for_spec_and_version(spec, version),
        BlockEnvExt::default(),
    );
    let tx = Recovered::new_unchecked(
        TxEnvelope::Legacy(TxLegacy {
            to: TxKind::Call(HANDLER),
            gas_limit: 1_000_000,
            ..Default::default()
        }),
        SENDER,
    );
    let (result, fuzzer) = executor.inspect_call(&tx, fuzzer);
    let result = result.unwrap();
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
fn evm2_fuzzer_observes_calls_and_replays_overrides() {
    for reenter in [false, true] {
        let observed = observations(run_evm2(fuzzer(reenter)));
        assert_eq!(observed.output, (U256::from(reenter), U256::from(5)));
        assert!(!observed.collected_values.is_empty());
        assert_eq!(observed.mapping_keys.len(), 1);
        assert_eq!(observed.observed_calls.len(), 1);
        assert_eq!(observed.override_depth, reenter.then_some(0));
        assert_eq!(observed.replays_left, reenter.then_some(0));
    }
}

#[test]
fn evm2_fuzzer_reports_exhausted_replay_without_panicking() {
    let fuzzer = fuzzer(true);
    fuzzer.call_generator.as_ref().unwrap().last_sequence.write().clear();
    let (_, fuzzer) = run_evm2(fuzzer);
    assert!(fuzzer.call_generator.unwrap().replay_exhausted);
}
