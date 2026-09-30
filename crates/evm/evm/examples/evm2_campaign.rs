//! Run with `cargo run -p foundry-evm --example evm2_campaign`.
//!
//! Exercises evm2's transaction boundary directly, using a contract that adds calldata to a
//! counter. This demonstrates engine state only; Foundry must also manage environment, cheatcodes,
//! and forks.

use alloy_consensus::{TxLegacy, transaction::Recovered};
use alloy_primitives::{Address, TxKind, U256};
use evm2::{
    BaseEvmTypes, Evm, Precompiles, SpecId,
    bytecode::Bytecode,
    env::BlockEnvExt,
    ethereum::{RecoveredTxEnvelope, TxEnvelope, ethereum_tx_registry},
    evm::{AccountInfo, InMemoryDB, SnapshotLogs},
    interpreter::op,
};

const CALLER: Address = Address::with_last_byte(0xaa);
const COUNTER: Address = Address::with_last_byte(0xbb);

fn main() {
    let mut database = InMemoryDB::default();
    // Equivalent to: counter += abi.decode(msg.data, (uint256)); return counter.
    let code = vec![
        op::PUSH0,
        op::CALLDATALOAD,
        op::PUSH0,
        op::SLOAD,
        op::ADD,
        op::DUP1,
        op::PUSH0,
        op::SSTORE,
        op::PUSH0,
        op::MSTORE,
        op::PUSH1,
        32,
        op::PUSH0,
        op::RETURN,
    ];
    database.insert_account_info(
        &COUNTER,
        AccountInfo::default().with_code(Bytecode::new_legacy(code.into())),
    );
    let spec = SpecId::CANCUN;
    let mut evm = Evm::<BaseEvmTypes>::new(
        spec,
        BlockEnvExt::default(),
        ethereum_tx_registry(spec),
        database,
        Precompiles::base(spec),
    );

    // Accept setup, then capture the starting point for every case or sequence.
    let setup = call(0, 10);
    let result = evm.transact(&setup).unwrap().commit();
    assert!(result.status);
    assert_eq!(U256::from_be_slice(&result.output), U256::from(10));
    let after_setup = evm.state().snapshot();

    // Fuzz cases: inspect each result, then discard its writes and caller nonce change.
    for amount in [1, 7, 42] {
        let tx = call(1, amount);
        let executed = evm.transact(&tx).unwrap();
        assert!(executed.result().status);
        assert_eq!(U256::from_be_slice(&executed.result().output), U256::from(10 + amount));
        let _ = executed.discard();
        assert_eq!(
            evm.state_mut().storage_slot_untracked(&COUNTER, &U256::ZERO).unwrap(),
            U256::from(10),
        );
    }

    // Invariant runs: accept handlers within a sequence, reset between sequences.
    for _ in 0..2 {
        // The backing database is unchanged here. This snapshot only restores evm2's memory.
        evm.state_mut().restore_snapshot(&after_setup, SnapshotLogs::Restore);
        for nonce in 1..=3 {
            let handler = call(nonce, 1);
            let result = evm.transact(&handler).unwrap().commit();
            assert!(result.status);
            assert_eq!(U256::from_be_slice(&result.output), U256::from(10 + nonce));

            // Model a predicate that writes: its changes must not reach the next handler.
            let predicate = call(nonce + 1, 100);
            let executed = evm.transact(&predicate).unwrap();
            assert!(executed.result().status);
            assert_eq!(U256::from_be_slice(&executed.result().output), U256::from(110 + nonce));
            let _ = executed.discard();
        }
        assert_eq!(
            evm.state_mut().storage_slot_untracked(&COUNTER, &U256::ZERO).unwrap(),
            U256::from(13),
        );
    }
}

fn call(nonce: u64, amount: u64) -> RecoveredTxEnvelope {
    Recovered::new_unchecked(
        TxEnvelope::Legacy(TxLegacy {
            nonce,
            gas_limit: 100_000,
            to: TxKind::Call(COUNTER),
            input: U256::from(amount).to_be_bytes_vec().into(),
            ..Default::default()
        }),
        CALLER,
    )
}
