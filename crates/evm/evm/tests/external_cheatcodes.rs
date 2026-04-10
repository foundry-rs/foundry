//! Integration tests for the external cheatcode plugin API.
//!
//! These tests build a real `Executor` with external cheatcode handlers registered,
//! then call the cheatcode address with custom selectors and verify the full dispatch
//! pipeline works end-to-end: unknown selector → external handler → return/revert.

use alloy_primitives::{Address, Bytes, U256, address};
use foundry_cheatcodes::{CheatcodeHost, ExternalCheatcode, ExternalCheatcodeOutcome};
use foundry_evm::executors::ExecutorBuilder;
use foundry_evm_core::{
    backend::Backend,
    constants::CHEATCODE_ADDRESS,
    evm::{EthEvmNetwork, EvmEnvFor, TxEnvFor},
};
use revm::DatabaseRef;
use std::sync::Arc;

fn default_env() -> (EvmEnvFor<EthEvmNetwork>, TxEnvFor<EthEvmNetwork>) {
    let mut evm_env = EvmEnvFor::<EthEvmNetwork>::default();
    evm_env.block_env.gas_limit = 16_777_216;
    (evm_env, Default::default())
}

fn default_backend() -> Backend<EthEvmNetwork> {
    Backend::spawn(None).unwrap()
}

const SENDER: Address = address!("0x1000000000000000000000000000000000000001");
const TARGET: Address = address!("0x2000000000000000000000000000000000000002");

/// External cheatcode that handles selector `0xdeadbeef` and returns uint256(42).
#[derive(Debug)]
struct ReturnHandler;

impl ExternalCheatcode for ReturnHandler {
    fn call(&self, _host: &mut dyn CheatcodeHost, calldata: &[u8]) -> ExternalCheatcodeOutcome {
        if calldata.len() >= 4 && calldata[..4] == [0xde, 0xad, 0xbe, 0xef] {
            let ret = U256::from(42).to_be_bytes_vec();
            ExternalCheatcodeOutcome::Return(ret)
        } else {
            ExternalCheatcodeOutcome::Unhandled
        }
    }
}

/// External cheatcode that handles selector `0xcafebabe` and reverts.
#[derive(Debug)]
struct RevertHandler;

impl ExternalCheatcode for RevertHandler {
    fn call(&self, _host: &mut dyn CheatcodeHost, calldata: &[u8]) -> ExternalCheatcodeOutcome {
        if calldata.len() >= 4 && calldata[..4] == [0xca, 0xfe, 0xba, 0xbe] {
            ExternalCheatcodeOutcome::Revert(foundry_cheatcodes::Error::from("custom revert"))
        } else {
            ExternalCheatcodeOutcome::Unhandled
        }
    }
}

/// External cheatcode that reads and writes storage via the host.
/// Selector `0xaabbccdd`: reads slot 0 of TARGET, writes slot 1 = slot 0 + 1, returns slot 0.
#[derive(Debug)]
struct StorageHandler;

impl ExternalCheatcode for StorageHandler {
    fn call(&self, host: &mut dyn CheatcodeHost, calldata: &[u8]) -> ExternalCheatcodeOutcome {
        if calldata.len() >= 4 && calldata[..4] == [0xaa, 0xbb, 0xcc, 0xdd] {
            let val = match host.load(TARGET, U256::ZERO) {
                Ok(v) => v,
                Err(e) => return ExternalCheatcodeOutcome::Revert(e),
            };
            if let Err(e) = host.store(TARGET, U256::from(1), val + U256::from(1)) {
                return ExternalCheatcodeOutcome::Revert(e);
            }
            ExternalCheatcodeOutcome::Return(val.to_be_bytes_vec())
        } else {
            ExternalCheatcodeOutcome::Unhandled
        }
    }
}

/// Selector `0x11111111`: sets TARGET balance to 123 ether, returns old balance.
#[derive(Debug)]
struct SetBalanceHandler;

impl ExternalCheatcode for SetBalanceHandler {
    fn call(&self, host: &mut dyn CheatcodeHost, calldata: &[u8]) -> ExternalCheatcodeOutcome {
        if calldata.len() >= 4 && calldata[..4] == [0x11, 0x11, 0x11, 0x11] {
            let old = match host.balance(TARGET) {
                Ok(v) => v,
                Err(e) => return ExternalCheatcodeOutcome::Revert(e),
            };
            let new_balance = U256::from(123_000_000_000_000_000_000u128); // 123 ether
            if let Err(e) = host.set_balance(TARGET, new_balance) {
                return ExternalCheatcodeOutcome::Revert(e);
            }
            ExternalCheatcodeOutcome::Return(old.to_be_bytes_vec())
        } else {
            ExternalCheatcodeOutcome::Unhandled
        }
    }
}

/// Selector `0x22222222`: sets TARGET code to `0x6001` (PUSH1 01), returns empty.
#[derive(Debug)]
struct SetCodeHandler;

impl ExternalCheatcode for SetCodeHandler {
    fn call(&self, host: &mut dyn CheatcodeHost, calldata: &[u8]) -> ExternalCheatcodeOutcome {
        if calldata.len() >= 4 && calldata[..4] == [0x22, 0x22, 0x22, 0x22] {
            if let Err(e) = host.set_code(TARGET, Bytes::from_static(&[0x60, 0x01])) {
                return ExternalCheatcodeOutcome::Revert(e);
            }
            ExternalCheatcodeOutcome::Return(Vec::new())
        } else {
            ExternalCheatcodeOutcome::Unhandled
        }
    }
}

/// Selector `0x33333333`: tries to store to precompile address 0x01. Should revert.
#[derive(Debug)]
struct PrecompileStoreHandler;

impl ExternalCheatcode for PrecompileStoreHandler {
    fn call(&self, host: &mut dyn CheatcodeHost, calldata: &[u8]) -> ExternalCheatcodeOutcome {
        if calldata.len() >= 4 && calldata[..4] == [0x33, 0x33, 0x33, 0x33] {
            let precompile = Address::with_last_byte(1);
            match host.store(precompile, U256::ZERO, U256::from(1)) {
                Ok(()) => ExternalCheatcodeOutcome::Return(Vec::new()),
                Err(e) => ExternalCheatcodeOutcome::Revert(e),
            }
        } else {
            ExternalCheatcodeOutcome::Unhandled
        }
    }
}

/// Selector `0x44444444`: tries to set_code on precompile address 0x02. Should revert.
#[derive(Debug)]
struct PrecompileSetCodeHandler;

impl ExternalCheatcode for PrecompileSetCodeHandler {
    fn call(&self, host: &mut dyn CheatcodeHost, calldata: &[u8]) -> ExternalCheatcodeOutcome {
        if calldata.len() >= 4 && calldata[..4] == [0x44, 0x44, 0x44, 0x44] {
            let precompile = Address::with_last_byte(2);
            match host.set_code(precompile, Bytes::from_static(&[0x60, 0x01])) {
                Ok(()) => ExternalCheatcodeOutcome::Return(Vec::new()),
                Err(e) => ExternalCheatcodeOutcome::Revert(e),
            }
        } else {
            ExternalCheatcodeOutcome::Unhandled
        }
    }
}

#[test]
fn external_handler_returns_data() {
    let (evm_env, tx_env) = default_env();
    let executor = ExecutorBuilder::<EthEvmNetwork>::default()
        .inspectors(|stack| stack.external_cheatcode(Arc::new(ReturnHandler)))
        .build(evm_env, tx_env, default_backend());

    let calldata = Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]);
    let result = executor.call_raw(SENDER, CHEATCODE_ADDRESS, calldata, U256::ZERO).unwrap();

    assert!(!result.reverted, "call should not revert");
    let expected = U256::from(42).to_be_bytes_vec();
    assert_eq!(result.result.as_ref(), expected.as_slice());
}

#[test]
fn external_handler_reverts() {
    let (evm_env, tx_env) = default_env();
    let executor = ExecutorBuilder::<EthEvmNetwork>::default()
        .inspectors(|stack| stack.external_cheatcode(Arc::new(RevertHandler)))
        .build(evm_env, tx_env, default_backend());

    let calldata = Bytes::from_static(&[0xca, 0xfe, 0xba, 0xbe]);
    let result = executor.call_raw(SENDER, CHEATCODE_ADDRESS, calldata, U256::ZERO).unwrap();

    assert!(result.reverted, "call should revert");
}

#[test]
fn unhandled_selector_reverts_with_unknown_cheatcode() {
    let (evm_env, tx_env) = default_env();
    let executor = ExecutorBuilder::<EthEvmNetwork>::default()
        .inspectors(|stack| stack.external_cheatcode(Arc::new(ReturnHandler)))
        .build(evm_env, tx_env, default_backend());

    // Selector that no handler recognizes
    let calldata = Bytes::from_static(&[0x11, 0x22, 0x33, 0x44]);
    let result = executor.call_raw(SENDER, CHEATCODE_ADDRESS, calldata, U256::ZERO).unwrap();

    assert!(result.reverted, "unhandled selector should revert");
}

#[test]
fn handler_chain_fallthrough() {
    let (evm_env, tx_env) = default_env();
    let executor = ExecutorBuilder::<EthEvmNetwork>::default()
        .inspectors(|stack| {
            stack
                .external_cheatcode(Arc::new(RevertHandler))
                .external_cheatcode(Arc::new(ReturnHandler))
        })
        .build(evm_env, tx_env, default_backend());

    // 0xdeadbeef: RevertHandler returns Unhandled, ReturnHandler handles it
    let calldata = Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]);
    let result = executor.call_raw(SENDER, CHEATCODE_ADDRESS, calldata, U256::ZERO).unwrap();

    assert!(!result.reverted);
    let expected = U256::from(42).to_be_bytes_vec();
    assert_eq!(result.result.as_ref(), expected.as_slice());
}

#[test]
fn handler_reads_and_writes_storage() {
    let (evm_env, tx_env) = default_env();
    let mut executor = ExecutorBuilder::<EthEvmNetwork>::default()
        .inspectors(|stack| stack.external_cheatcode(Arc::new(StorageHandler)))
        .build(evm_env, tx_env, default_backend());

    // Set slot 0 of TARGET to 99
    executor.set_balance(TARGET, U256::from(1)).unwrap(); // ensure account exists
    executor.backend_mut().insert_account_storage(TARGET, U256::ZERO, U256::from(99)).unwrap();

    // Call the stateful handler: reads slot 0, writes slot 1 = slot 0 + 1
    let calldata = Bytes::from_static(&[0xaa, 0xbb, 0xcc, 0xdd]);
    let result = executor.transact_raw(SENDER, CHEATCODE_ADDRESS, calldata, U256::ZERO).unwrap();

    assert!(!result.reverted, "stateful handler should not revert");

    // Verify slot 1 was written (99 + 1 = 100)
    let slot1 = executor.backend().storage_ref(TARGET, U256::from(1)).unwrap();
    assert_eq!(slot1, U256::from(100));
}

#[test]
fn builtin_cheatcodes_still_work_with_external_handlers() {
    let (evm_env, tx_env) = default_env();
    let executor = ExecutorBuilder::<EthEvmNetwork>::default()
        .inspectors(|stack| stack.external_cheatcode(Arc::new(ReturnHandler)))
        .build(evm_env, tx_env, default_backend());

    // vm.getNonce(address) — selector 0x2d0335ab — is a built-in cheatcode
    // It should still work even with external handlers registered
    let mut calldata = vec![0x2d, 0x03, 0x35, 0xab]; // getNonce selector
    calldata.extend_from_slice(&[0u8; 12]); // left-pad address
    calldata.extend_from_slice(SENDER.as_slice());
    let result =
        executor.call_raw(SENDER, CHEATCODE_ADDRESS, Bytes::from(calldata), U256::ZERO).unwrap();

    assert!(!result.reverted, "built-in cheatcode should still work");
}

#[test]
fn handler_sets_balance() {
    let (evm_env, tx_env) = default_env();
    let mut executor = ExecutorBuilder::<EthEvmNetwork>::default()
        .inspectors(|stack| stack.external_cheatcode(Arc::new(SetBalanceHandler)))
        .build(evm_env, tx_env, default_backend());

    let calldata = Bytes::from_static(&[0x11, 0x11, 0x11, 0x11]);
    let result = executor.transact_raw(SENDER, CHEATCODE_ADDRESS, calldata, U256::ZERO).unwrap();

    assert!(!result.reverted);

    let balance = executor.get_balance(TARGET).unwrap();
    assert_eq!(balance, U256::from(123_000_000_000_000_000_000u128));
}

#[test]
fn handler_sets_code() {
    let (evm_env, tx_env) = default_env();
    let mut executor = ExecutorBuilder::<EthEvmNetwork>::default()
        .inspectors(|stack| stack.external_cheatcode(Arc::new(SetCodeHandler)))
        .build(evm_env, tx_env, default_backend());

    let calldata = Bytes::from_static(&[0x22, 0x22, 0x22, 0x22]);
    let result = executor.transact_raw(SENDER, CHEATCODE_ADDRESS, calldata, U256::ZERO).unwrap();

    assert!(!result.reverted);

    let code = executor.backend().basic_ref(TARGET).unwrap().unwrap();
    assert_eq!(code.code.unwrap().original_bytes(), Bytes::from_static(&[0x60, 0x01]));
}

#[test]
fn store_to_precompile_reverts() {
    let (evm_env, tx_env) = default_env();
    let executor = ExecutorBuilder::<EthEvmNetwork>::default()
        .inspectors(|stack| stack.external_cheatcode(Arc::new(PrecompileStoreHandler)))
        .build(evm_env, tx_env, default_backend());

    let calldata = Bytes::from_static(&[0x33, 0x33, 0x33, 0x33]);
    let result = executor.call_raw(SENDER, CHEATCODE_ADDRESS, calldata, U256::ZERO).unwrap();

    assert!(result.reverted, "store to precompile should revert");
}

#[test]
fn set_code_on_precompile_reverts() {
    let (evm_env, tx_env) = default_env();
    let executor = ExecutorBuilder::<EthEvmNetwork>::default()
        .inspectors(|stack| stack.external_cheatcode(Arc::new(PrecompileSetCodeHandler)))
        .build(evm_env, tx_env, default_backend());

    let calldata = Bytes::from_static(&[0x44, 0x44, 0x44, 0x44]);
    let result = executor.call_raw(SENDER, CHEATCODE_ADDRESS, calldata, U256::ZERO).unwrap();

    assert!(result.reverted, "set_code on precompile should revert");
}
