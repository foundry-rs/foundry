//! Backend behavior shared by every network family, over in-memory backing databases.

use super::*;
use crate::evm::{
    EvmEnv, TransactionRequestFor,
    tests::{ETHER, STORE, STORE_ADDRESS, balance, db_with, nonce, slot, store_request},
};
use alloy_consensus::{Signed, TxEnvelope, TxLegacy};
use alloy_network::{AnyHeader, AnyRpcHeader, AnyTxEnvelope};
use alloy_primitives::{Bytes, Signature};
use alloy_rpc_types::{Block, Transaction as RpcTransaction};
use alloy_serde::WithOtherFields;
use evm2::NoopInspector;

const CHAIN_ID: u64 = 31337;

/// Registers a fork over `db` that is not RPC-backed.
fn memory_fork<FEN: FoundryEvmNetwork>(
    backend: &mut Backend<FEN>,
    db: impl BackingDb,
    env: EvmEnvFor<FEN>,
) -> LocalForkId {
    let id = backend.lifecycle.forks().len() as u64;
    let info = ForkInfo::new(ForkId::new("memory", Some(id)), "memory", Some(id));
    backend.lifecycle.create_fork(info, db, env)
}

/// A mined legacy transaction of `caller` that stores `value` through the `STORE` contract.
fn rpc_store_tx(caller: Address, nonce: u64, value: u64) -> AnyRpcTransaction {
    let tx = TxLegacy {
        nonce,
        gas_limit: 1_000_000,
        to: TxKind::Call(STORE_ADDRESS),
        input: B256::from(U256::from(value)).into(),
        ..Default::default()
    };
    let hash = B256::from(U256::from(value));
    let signed = Signed::new_unchecked(tx, Signature::new(U256::ONE, U256::ONE, false), hash);
    AnyRpcTransaction::new(WithOtherFields::new(RpcTransaction {
        inner: Recovered::new_unchecked(
            AnyTxEnvelope::Ethereum(TxEnvelope::Legacy(signed)),
            caller,
        ),
        block_hash: None,
        block_number: Some(1),
        transaction_index: None,
        effective_gas_price: None,
        block_timestamp: None,
    }))
}

fn rpc_block(transactions: Vec<AnyRpcTransaction>) -> AnyRpcBlock {
    let header = AnyHeader { number: 1, ..Default::default() };
    AnyRpcBlock::new(
        Block::new(
            AnyRpcHeader::from_sealed(header.seal(B256::with_last_byte(1))),
            BlockTransactions::Full(transactions),
        )
        .into(),
    )
}

/// Executions, accepted-state access, allocs, snapshots, persistent accounts, fork selection,
/// nested transactions, revert diagnostics and rolling to a replayed block position.
fn backend_suite<FEN: FoundryEvmNetwork>(spec: SpecFor<FEN>)
where
    TransactionRequestFor<FEN>: Default,
{
    let caller = Address::with_last_byte(0xCA);
    let factory = EvmFactoryFor::<FEN>::default();
    let env = EvmEnv::new(spec, factory.version(spec, CHAIN_ID), Default::default());
    let store = |nonce, value| {
        FEN::tx_from_request(store_request::<FEN>(caller, STORE_ADDRESS, nonce, value, CHAIN_ID))
            .unwrap()
    };
    let mut backend = Backend::<FEN>::with_db(db_with(&[(caller, ETHER)], 1), env);
    let boundary = ExecutionPhase::Boundary;

    // Accepted state is readable and writable between executions.
    assert_eq!(backend.storage(STORE_ADDRESS, U256::ZERO).unwrap(), U256::from(1));
    backend.insert_account_storage(STORE_ADDRESS, U256::ZERO, U256::from(2));
    assert_eq!(backend.storage(STORE_ADDRESS, U256::ZERO).unwrap(), U256::from(2));
    let other = Address::with_last_byte(0xC1);
    backend.insert_account_info(other, AccountInfo::default().with_balance(U256::from(3)));
    assert_eq!(backend.basic(other).unwrap().unwrap().balance, U256::from(3));
    let code = backend.basic(STORE_ADDRESS).unwrap().unwrap().code.unwrap();
    assert_eq!(code.original_bytes(), Bytes::from_static(STORE));
    backend.replace_account_storage(other, [(U256::from(1), U256::from(4))]);
    assert_eq!(backend.storage(other, U256::from(1)).unwrap(), U256::from(4));

    // Calls discard their state; commits accept it.
    assert!(backend.inspect_call(&store(0, 7), NoopInspector::default()).unwrap().status);
    assert_eq!(backend.storage(STORE_ADDRESS, U256::ZERO).unwrap(), U256::from(2));
    assert_eq!(backend.basic(caller).unwrap().unwrap().nonce, 0);
    assert!(backend.inspect_commit(&store(0, 7), NoopInspector::default()).unwrap().status);
    assert_eq!(backend.storage(STORE_ADDRESS, U256::ZERO).unwrap(), U256::from(7));
    assert_eq!(backend.basic(caller).unwrap().unwrap().nonce, 1);

    // A clone-on-write backend borrows until its first mutation, then leaves the original alone.
    let mut cow = CowBackend::new_borrowed(&backend);
    cow.initialize(caller, TxKind::Call(STORE_ADDRESS));
    assert!(cow.inspect_call(&store(1, 8), NoopInspector::default()).unwrap().status);
    assert!(!cow.is_owned());
    assert!(cow.inspect_commit(&store(1, 8), NoopInspector::default()).unwrap().status);
    assert!(cow.is_owned());
    assert!(cow.has_cheatcode_access(&caller) && cow.is_persistent(&STORE_ADDRESS));
    assert_eq!(cow.storage(STORE_ADDRESS, U256::ZERO).unwrap(), U256::from(8));
    assert_eq!(backend.storage(STORE_ADDRESS, U256::ZERO).unwrap(), U256::from(7));
    assert!(!backend.has_cheatcode_access(&caller));

    // Allocs are journaled writes of the running transaction.
    let mut evm = backend.create_evm();
    let alloc = Address::with_last_byte(0xA1);
    let account = GenesisAccount {
        balance: U256::from(9),
        nonce: Some(4),
        code: Some(Bytes::from_static(STORE)),
        storage: Some([(B256::ZERO, B256::with_last_byte(5))].into_iter().collect()),
        private_key: None,
    };
    backend.load_allocs(&[(alloc, account)].into_iter().collect(), &mut evm).unwrap();
    assert_eq!((balance(&mut evm, alloc), nonce(&mut evm, alloc)), (U256::from(9), 4));
    let alloc_slot = evm.state_mut().storage_slot_untracked(&alloc, &U256::ZERO).unwrap();
    assert_eq!(alloc_slot, U256::from(5));
    evm.state_mut().commit_transaction();

    // Snapshot reverts restore state and record a failure marked before the revert.
    let snapshot = backend.snapshot_state(&mut evm);
    assert!(evm.transact(&store(1, 10)).unwrap().commit().status);
    assert_eq!(slot(&mut evm), U256::from(10));
    evm.state_mut().storage_slot(&CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT).unwrap().set(U256::ONE);
    evm.state_mut().commit_transaction();
    let remove = RevertStateSnapshotAction::RevertRemove;
    assert_eq!(
        backend.revert_state(snapshot, &mut evm, remove, boundary),
        Some(EnvTransition::Applied)
    );
    assert_eq!((slot(&mut evm), nonce(&mut evm, caller)), (U256::from(7), 1));
    assert!(backend.has_state_snapshot_failure());
    assert_eq!(backend.revert_state(snapshot, &mut evm, remove, boundary), None);

    // Persistent accounts follow fork selection; everything else stays per fork.
    let fork = memory_fork(&mut backend, db_with(&[(caller, ETHER)], 100), env);
    assert!(backend.add_persistent_account(alloc));
    assert_eq!(backend.select_fork(fork, &mut evm, boundary).unwrap(), EnvTransition::Applied);
    assert_eq!(backend.active_fork_id(), Some(fork));
    assert_eq!(slot(&mut evm), U256::from(100));
    assert_eq!((balance(&mut evm, alloc), nonce(&mut evm, caller)), (U256::from(9), 0));

    // A nested transaction merges its writes into the running one.
    let result = backend.transact_from_tx(&store(0, 101), &mut evm, None, None).unwrap();
    assert!(result.status);
    assert_eq!((slot(&mut evm), nonce(&mut evm, caller)), (U256::from(101), 1));
    evm.state_mut().commit_transaction();

    // A call to a contract that only exists on another fork is diagnosed.
    let elsewhere = Address::with_last_byte(0xEE);
    let mut other_db = db_with(&[], 0);
    let mut info = AccountInfo::default().with_code(Bytecode::new_raw(Bytes::from_static(STORE)));
    other_db.insert_contract(&mut info);
    other_db.insert_account_info(&elsewhere, info);
    let other_fork = memory_fork(&mut backend, other_db, env);
    assert!(matches!(
        backend.diagnose_revert(elsewhere, &mut evm),
        Some(RevertDiagnostic::ContractExistsOnOtherForks { available_on, .. })
            if available_on == [other_fork]
    ));
    assert!(backend.diagnose_revert(STORE_ADDRESS, &mut evm).is_none());

    // Rolling to a transaction resumes on the block's state replayed up to it; persistent
    // accounts survive the roll.
    let rolled = db_with(&[(caller, ETHER)], 200);
    let block = rpc_block(vec![rpc_store_tx(caller, 0, 201), rpc_store_tx(caller, 1, 202)]);
    let cache = backend.replay_until(&rolled, env, &block, 1, None, CHAIN_ID).unwrap();
    let transition =
        backend.lifecycle.roll_fork_with(&mut evm, Some(fork), rolled, env, Some(cache), boundary);
    assert_eq!(transition.unwrap(), EnvTransition::Applied);
    assert_eq!((slot(&mut evm), nonce(&mut evm, caller)), (U256::from(201), 1));
    assert_eq!(balance(&mut evm, alloc), U256::from(9));

    // Parking accepts the running state into the active fork.
    backend.park(evm);
    assert_eq!(backend.storage(STORE_ADDRESS, U256::ZERO).unwrap(), U256::from(201));
    assert_eq!(backend.basic(alloc).unwrap().unwrap().balance, U256::from(9));
}

#[test]
fn eth_backend() {
    backend_suite::<EthEvmNetwork>(evm2::SpecId::PRAGUE);
}

#[test]
fn tempo_backend() {
    backend_suite::<crate::evm::TempoEvmNetwork>(tempo_hardfork::TempoHardfork::default());
}

#[cfg(feature = "optimism")]
#[test]
fn op_backend() {
    backend_suite::<crate::evm::OpEvmNetwork>(op_evm2::OpSpecId::default());
}

#[cfg(feature = "monad")]
#[test]
fn monad_backend() {
    backend_suite::<crate::evm::MonadEvmNetwork>(monad_evm2::MonadHardfork::default());
}

#[cfg(feature = "base")]
#[test]
fn base_backend() {
    backend_suite::<crate::evm::BaseEvmNetwork>(base_evm2::BaseSpecId::new(
        base_evm2::BaseUpgrade::Beryl,
    ));
}

#[test]
fn select_fork_carries_running_spec_and_fee_policy() {
    let factory = EvmFactoryFor::<EthEvmNetwork>::default();
    let env = |spec| EvmEnv::new(spec, factory.version(spec, CHAIN_ID), Default::default());
    let mut backend = Backend::<EthEvmNetwork>::with_db(db_with(&[], 0), env(evm2::SpecId::PRAGUE));
    let fork = memory_fork(&mut backend, db_with(&[], 0), env(evm2::SpecId::LONDON));
    backend.evm_env_mut().set_feature(EvmFeatures::FEE_CHARGE, false);
    let mut evm = backend.create_evm();
    let _ = backend.select_fork(fork, &mut evm, ExecutionPhase::Boundary).unwrap();
    assert_eq!(evm.config_spec_id(), evm2::SpecId::PRAGUE);
    assert!(!evm.version().features.contains(EvmFeatures::FEE_CHARGE));
}

#[test]
fn fork_loaded_storage_lifetime() {
    let factory = EvmFactoryFor::<EthEvmNetwork>::default();
    let spec = evm2::SpecId::PRAGUE;
    let env = EvmEnv::new(spec, factory.version(spec, CHAIN_ID), Default::default());
    let mut backend = Backend::<EthEvmNetwork>::with_db(db_with(&[], 0), env);
    let first = memory_fork(&mut backend, db_with(&[], 17), env);
    let second = memory_fork(&mut backend, db_with(&[], 97), env);
    let phase = ExecutionPhase::InFlight;
    let mut evm = backend.create_evm();
    let _ = backend.select_fork(first, &mut evm, phase).unwrap();
    evm.state_mut().storage_slot(&STORE_ADDRESS, U256::ZERO).unwrap();
    evm.state_mut().storage_slot(&STORE_ADDRESS, U256::ONE).unwrap().set(U256::from(23));
    backend.add_persistent_account(STORE_ADDRESS);

    let _ = backend.select_fork(second, &mut evm, phase).unwrap();
    assert_eq!(
        backend.lifecycle().loaded_storage(evm.state(), &STORE_ADDRESS),
        [U256::ZERO, U256::ONE]
    );
    assert_eq!(slot(&mut evm), U256::from(17));

    backend.remove_persistent_account(&STORE_ADDRESS);
    evm.state_mut().storage_slot(&STORE_ADDRESS, U256::ZERO).unwrap().set(U256::from(97));
    let _ = backend.select_fork(first, &mut evm, phase).unwrap();
    assert_eq!(
        backend.lifecycle().loaded_storage(evm.state(), &STORE_ADDRESS),
        [U256::ZERO, U256::ONE]
    );
    assert_eq!(slot(&mut evm), U256::from(17));

    let snapshot = backend.snapshot_state(&mut evm);
    evm.state_mut().storage_slot(&STORE_ADDRESS, U256::from(2)).unwrap();
    let _ = backend.select_fork(second, &mut evm, phase).unwrap();
    let _ = backend
        .revert_state(snapshot, &mut evm, RevertStateSnapshotAction::RevertRemove, phase)
        .unwrap();
    assert_eq!(
        backend.lifecycle().loaded_storage(evm.state(), &STORE_ADDRESS),
        [U256::ZERO, U256::ONE]
    );

    backend.park(evm);
    let mut evm = backend.create_evm();
    assert!(backend.lifecycle().loaded_storage(evm.state(), &STORE_ADDRESS).is_empty());
    let _ = backend.select_fork(second, &mut evm, phase).unwrap();
    assert!(backend.lifecycle().loaded_storage(evm.state(), &STORE_ADDRESS).is_empty());
    assert_eq!(slot(&mut evm), U256::from(97));
}

#[test]
fn fork_carries_created_accounts() {
    let factory = EvmFactoryFor::<EthEvmNetwork>::default();
    let spec = evm2::SpecId::PRAGUE;
    let env = EvmEnv::new(spec, factory.version(spec, CHAIN_ID), Default::default());
    let persistent = Address::with_last_byte(0xC0);
    let local = Address::with_last_byte(0xC1);
    let changed = Address::with_last_byte(0xC2);
    let on_fork = Address::with_last_byte(0xC3);
    let mut fork_db = db_with(&[(changed, 5)], 0);
    // The fork holds storage at an address the transaction creates locally.
    fork_db.insert_account_storage(&persistent, &U256::ZERO, &U256::from(17));
    let mut backend = Backend::<EthEvmNetwork>::with_db(db_with(&[], 0), env);
    let first = memory_fork(&mut backend, fork_db, env);
    let second = memory_fork(&mut backend, db_with(&[], 0), env);
    let phase = ExecutionPhase::InFlight;
    let mut evm = backend.create_evm();
    let features = evm.version().features;
    for address in [persistent, local] {
        evm.state_mut().create_account(&CALLER, &address, &U256::ZERO, features).unwrap().unwrap();
    }
    evm.state_mut().storage_slot(&persistent, U256::ONE).unwrap().set(U256::from(23));
    evm.state_mut().account(&changed).unwrap().set_balance(U256::from(9));
    backend.add_persistent_account(persistent);

    let _ = backend.select_fork(first, &mut evm, phase).unwrap();
    assert_eq!(nonce(&mut evm, local), 1);
    let storage = |evm: &mut FoundryEvmFor<'_, EthEvmNetwork>, key| {
        evm.state_mut().storage_slot_untracked(&persistent, &U256::from(key)).unwrap()
    };
    assert_eq!(storage(&mut evm, 0), U256::ZERO);
    assert_eq!(storage(&mut evm, 1), U256::from(23));
    assert_eq!(balance(&mut evm, changed), U256::from(5));

    evm.state_mut().create_account(&CALLER, &on_fork, &U256::ZERO, features).unwrap().unwrap();
    let _ =
        backend.lifecycle.roll_fork(&mut evm, Some(first), db_with(&[], 0), env, phase).unwrap();
    assert_eq!(nonce(&mut evm, on_fork), 1);
    assert_eq!(nonce(&mut evm, local), 1);

    let _ = backend.select_fork(second, &mut evm, phase).unwrap();
    assert_eq!(nonce(&mut evm, on_fork), 0);
}
