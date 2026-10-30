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
use evm2::{NoopInspector, evm::InMemoryDB};

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
    assert!(backend.create_evm().transact(&store(0, 7)).unwrap().result().status);
    assert_eq!(backend.storage(STORE_ADDRESS, U256::ZERO).unwrap(), U256::from(2));
    assert_eq!(backend.basic(caller).unwrap().unwrap().nonce, 0);
    assert!(backend.inspect_commit(&store(0, 7), NoopInspector::default()).unwrap().status);
    assert_eq!(backend.storage(STORE_ADDRESS, U256::ZERO).unwrap(), U256::from(7));
    assert_eq!(backend.basic(caller).unwrap().unwrap().nonce, 1);

    // A clone-on-write backend borrows until its first mutation, then leaves the original alone.
    let mut cow = CowBackend::new_borrowed(&backend);
    cow.initialize(caller, TxKind::Call(STORE_ADDRESS));
    assert!(cow.create_evm().transact(&store(1, 8)).unwrap().result().status);
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
        ..Default::default()
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
    assert!(backend.revert_state(snapshot, &mut evm, remove, boundary).is_some());
    assert_eq!((slot(&mut evm), nonce(&mut evm, caller)), (U256::from(7), 1));
    assert!(backend.has_state_snapshot_failure());
    assert!(backend.revert_state(snapshot, &mut evm, remove, boundary).is_none());

    // Persistent accounts follow fork selection; everything else stays per fork.
    let fork = memory_fork(&mut backend, db_with(&[(caller, ETHER)], 100), env);
    assert!(backend.add_persistent_account(alloc));
    backend.select_fork(fork, &mut evm, boundary).unwrap();
    assert_eq!(backend.active_fork_id(), Some(fork));
    assert_eq!(slot(&mut evm), U256::from(100));
    assert_eq!((balance(&mut evm, alloc), nonce(&mut evm, caller)), (U256::from(9), 0));

    // A nested transaction merges its writes into the running one.
    let result = backend.transact_from_tx(&store(0, 101), &mut evm, None, None).unwrap().0;
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
    backend.lifecycle.roll_fork(&mut evm, Some(fork), rolled, env, Some(cache), boundary).unwrap();
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
fn select_fork_carries_running_spec_and_fee_policy() {
    let factory = EvmFactoryFor::<EthEvmNetwork>::default();
    let env = |spec| EvmEnv::new(spec, factory.version(spec, CHAIN_ID), Default::default());
    let mut backend = Backend::<EthEvmNetwork>::with_db(db_with(&[], 0), env(evm2::SpecId::PRAGUE));
    let fork = memory_fork(&mut backend, db_with(&[], 0), env(evm2::SpecId::LONDON));
    backend.evm_env_mut().set_feature(EvmFeatures::FEE_CHARGE, false);
    let mut evm = backend.create_evm();
    backend.select_fork(fork, &mut evm, ExecutionPhase::Boundary).unwrap();
    assert_eq!(evm.config_spec_id(), evm2::SpecId::PRAGUE);
    assert!(!evm.version().features.contains(EvmFeatures::FEE_CHARGE));
}

/// Ethereum replay skips protocol system envelopes it cannot represent, but still rejects
/// unknown non-system transaction types.
#[test]
fn ethereum_replay_skips_unknown_system_envelopes_before_conversion() {
    use alloy_network::{AnyTxType, UnknownTxEnvelope, UnknownTypedTransaction};

    let transaction = |ty| {
        let unknown = AnyTxEnvelope::Unknown(UnknownTxEnvelope {
            hash: B256::ZERO,
            inner: UnknownTypedTransaction {
                ty: AnyTxType(ty),
                fields: Default::default(),
                memo: Default::default(),
            },
        });
        AnyRpcTransaction::new(WithOtherFields::new(RpcTransaction {
            inner: Recovered::new_unchecked(unknown, Address::with_last_byte(0x42)),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            effective_gas_price: None,
            block_timestamp: None,
        }))
    };
    let factory = EvmFactoryFor::<EthEvmNetwork>::default();
    let spec = evm2::SpecId::PRAGUE;
    let env = EvmEnv::new(spec, factory.version(spec, CHAIN_ID), Default::default());
    let backend = Backend::<EthEvmNetwork>::with_db(db_with(&[], 0), env);
    let db = db_with(&[], 0);

    let block = rpc_block(vec![transaction(SYSTEM_TRANSACTION_TYPE), transaction(0xff)]);
    assert!(backend.replay_until(&db, env, &block, 1, None, CHAIN_ID).is_ok());
    assert!(backend.replay_until(&db, env, &block, 2, None, CHAIN_ID).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn refresh_fork_account_updates_loaded_journals() {
    use alloy_provider::Provider;

    let (api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
    let target = alloy_primitives::address!("0x0000000000000000000000000000000000001331");
    let code = Bytes::from_static(&[0x60, 0x2a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);

    let block_number = handle.http_provider().get_block_number().await.unwrap();
    let mut evm_opts = EvmOpts {
        fork_url: Some(handle.http_endpoint()),
        fork_block_number: Some(block_number),
        ..Default::default()
    };
    evm_opts.infer_network_from_fork().await.unwrap();
    let fork = evm_opts.get_fork(&Default::default(), 31_337, Some(block_number)).unwrap();
    let backend =
        Backend::<EthEvmNetwork>::spawn(Some(fork), evm_opts.local_evm_env::<EthEvmNetwork>())
            .unwrap();

    // The running transaction loaded the account and changed its balance.
    let mut evm = backend.create_evm();
    evm.state_mut().account(&target).unwrap().set_balance(U256::ONE);
    assert_eq!(
        evm.read_account_info(&target).unwrap().unwrap().code_hash,
        alloy_primitives::KECCAK256_EMPTY
    );

    api.anvil_set_code(target, code.clone()).await.unwrap();
    backend.refresh_fork_account(target, ForkAccountField::Code, &mut evm).unwrap();

    let refreshed = evm.read_account_info(&target).unwrap().unwrap();
    assert_eq!(refreshed.code_hash, alloy_primitives::keccak256(&code));
    assert_eq!(refreshed.balance, U256::ONE);
    let refreshed_code = evm.state_mut().account(&target).unwrap().load_code().unwrap();
    assert_eq!(refreshed_code.original_bytes(), code);
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_execution_scopes_release_their_registry() {
    let (_api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
    let request = CreateFork {
        url: handle.http_endpoint(),
        enable_caching: false,
        evm_opts: EvmOpts { fork_url: Some(handle.http_endpoint()), ..Default::default() },
    };
    let env = EvmOpts::default().local_evm_env::<EthEvmNetwork>();
    let seed = Backend::<EthEvmNetwork>::spawn(Some(request.clone()), env).unwrap();
    let initial = seed.fork_info(seed.active_fork_id().unwrap()).unwrap().id.clone();
    let shared = seed.forks.get_fork(initial.clone()).unwrap().unwrap();
    let mut execution = seed.clone_with_fork_scope().unwrap();
    let inherited = execution.forks.get_fork(initial.clone()).unwrap().unwrap();
    assert!(Arc::ptr_eq(&shared.data(), &inherited.data()));

    let local = execution.create_fork(request).unwrap();
    let local = execution.fork_info(local).unwrap().id.clone();
    assert!(execution.forks.get_fork(local.clone()).unwrap().is_some());
    assert!(seed.forks.get_fork(local.clone()).unwrap().is_none());
    let next = seed.clone_with_fork_scope().unwrap();
    assert!(next.forks.get_fork(local).unwrap().is_none());
    let (_other_api, other) = anvil::spawn(anvil::NodeConfig::test()).await;
    let unique = execution
        .create_fork(CreateFork {
            url: other.http_endpoint(),
            enable_caching: false,
            evm_opts: EvmOpts { fork_url: Some(other.http_endpoint()), ..Default::default() },
        })
        .unwrap();
    let unique = execution.fork_info(unique).unwrap().id.clone();
    let unique = execution.forks.get_fork(unique).unwrap().unwrap();
    let cache = Arc::downgrade(&unique.data());
    drop(unique);
    drop(execution);
    assert!(cache.upgrade().is_none(), "execution-owned remote cache must be released");
    drop(seed);

    // The child retains the original handler even after its owner is gone.
    assert!(next.forks.get_fork(initial).unwrap().is_some());
    assert!(
        next.basic(handle.dev_accounts().next().unwrap()).unwrap().unwrap().balance > U256::ZERO
    );
}

/// A persistent account a transaction on an inactive fork changes takes that fork's code with its
/// code hash, though the running transaction's database holds other code for it.
#[test]
fn persistent_account_takes_inactive_fork_code() {
    let factory = EvmFactoryFor::<EthEvmNetwork>::default();
    let spec = evm2::SpecId::CANCUN;
    let env = EvmEnv::new(spec, factory.version(spec, CHAIN_ID), Default::default());
    let caller = Address::with_last_byte(0xCA);
    let mut local = InMemoryDB::default();
    local.insert_account_info(&caller, AccountInfo::default().with_balance(U256::from(ETHER)));
    let mut info = AccountInfo::default().with_code(Bytecode::new_legacy(Bytes::from_static(&[0])));
    local.insert_contract(&mut info);
    local.insert_account_info(&STORE_ADDRESS, info);
    let mut backend = Backend::<EthEvmNetwork>::with_db(local, env);
    let fork = memory_fork(&mut backend, db_with(&[(caller, ETHER)], 1), env);
    let mut evm = backend.create_evm();
    backend.add_persistent_account(STORE_ADDRESS);

    let request = store_request::<EthEvmNetwork>(caller, STORE_ADDRESS, 0, 5, CHAIN_ID);
    let tx = EthEvmNetwork::tx_from_request(request).unwrap();
    let prepared = backend.lifecycle.prepare_on_inactive(&mut evm, Some(fork), tx, None, None);
    let executed = prepared.unwrap().execute(&mut evm, None, false).unwrap();
    assert!(backend.lifecycle.store_executed(executed).status);

    assert_eq!(slot(&mut evm), U256::from(5));
    let mut account = evm.state_mut().account(&STORE_ADDRESS).unwrap();
    let code = account.load_code().unwrap();
    assert_eq!(code.original_bytes(), Bytes::from_static(STORE));
    assert_eq!(account.code_hash(), code.hash_slow());
}
