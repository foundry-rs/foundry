use super::*;
use crate::{backend::RevertStateSnapshotAction, fork::ForkId};
use alloy_consensus::{Transaction as _, TxEip1559};
use alloy_eips::{BlockNumHash, Encodable2718};
use alloy_network::{
    AnyTxEnvelope, AnyTxType, TxSignerSync, UnknownTxEnvelope, UnknownTypedTransaction,
};
use alloy_primitives::{B256, Bytes, TxKind, U256};
use alloy_rpc_types::Transaction as RpcTransaction;
use alloy_serde::WithOtherFields;
use alloy_signer_local::PrivateKeySigner;
use evm2::{EvmFeatures, SpecId, bytecode::Bytecode, evm::InMemoryDB};

const GWEI: u128 = 1_000_000_000;
pub(crate) const ETHER: u128 = 1_000_000_000_000_000_000;

/// `sstore(0, calldataload(0))`.
pub(crate) const STORE: &[u8] = &[0x60, 0x00, 0x35, 0x60, 0x00, 0x55, 0x00];
/// `sstore(0, calldataload(0)); revert(0, 0)`.
const STORE_THEN_REVERT: &[u8] = &[0x60, 0x00, 0x35, 0x60, 0x00, 0x55, 0x60, 0x00, 0x80, 0xfd];

pub(crate) const STORE_ADDRESS: Address = Address::with_last_byte(0x5A);
pub(crate) const REVERT_ADDRESS: Address = Address::with_last_byte(0x5B);

pub(crate) fn db_with(accounts: &[(Address, u128)], slot: u64) -> InMemoryDB {
    let mut db = InMemoryDB::default();
    for (address, balance) in accounts {
        db.insert_account_info(address, AccountInfo::default().with_balance(U256::from(*balance)));
    }
    for (address, code) in [(STORE_ADDRESS, STORE), (REVERT_ADDRESS, STORE_THEN_REVERT)] {
        let mut info =
            AccountInfo::default().with_code(Bytecode::new_legacy(Bytes::from_static(code)));
        db.insert_contract(&mut info);
        db.insert_account_info(&address, info);
    }
    db.insert_account_storage(&STORE_ADDRESS, &U256::ZERO, &U256::from(slot));
    db
}

pub(crate) fn balance<T: EvmTypes>(evm: &mut Evm<'_, T>, address: Address) -> U256 {
    evm.read_account_info(&address).unwrap().map(|info| info.balance).unwrap_or_default()
}

pub(crate) fn nonce<T: EvmTypes>(evm: &mut Evm<'_, T>, address: Address) -> u64 {
    evm.read_account_info(&address).unwrap().map(|info| info.nonce).unwrap_or_default()
}

pub(crate) fn slot<T: EvmTypes>(evm: &mut Evm<'_, T>) -> U256 {
    evm.state_mut().storage_slot_untracked(&STORE_ADDRESS, &U256::ZERO).unwrap()
}

/// A complete, fee-free request that stores `value` through the `STORE` contract.
pub(crate) fn store_request<FEN: FoundryEvmNetwork>(
    from: Address,
    to: Address,
    nonce: u64,
    value: u64,
    chain_id: u64,
) -> TransactionRequestFor<FEN>
where
    TransactionRequestFor<FEN>: Default,
{
    TransactionRequestFor::<FEN>::default()
        .with_from(from)
        .with_kind(TxKind::Call(to))
        .with_input(B256::from(U256::from(value)))
        .with_nonce(nonce)
        .with_chain_id(chain_id)
        .with_gas_limit(1_000_000)
        .with_max_fee_per_gas(0)
        .with_max_priority_fee_per_gas(0)
}

/// Lifecycle behavior every network family shares: native transactions, commit and revert,
/// snapshots, forks with persistent accounts, rolling, nested frames and nested transactions.
fn lifecycle_suite<FEN: FoundryEvmNetwork>(factory: EvmFactoryFor<FEN>, spec: SpecFor<FEN>)
where
    TransactionRequestFor<FEN>: Default,
{
    let chain_id = 31337;
    let signer = PrivateKeySigner::random();
    let caller = signer.address();
    let request =
        |to, nonce, value, chain_id| store_request::<FEN>(caller, to, nonce, value, chain_id);
    let env = EvmEnv::new(spec, factory.version(spec, chain_id), Default::default());
    let mut lifecycle =
        ExecutionLifecycle::new(factory.clone(), db_with(&[(caller, ETHER)], 1), env);
    let mut evm = lifecycle.create_evm();

    // An unsigned request converts once and commits.
    let tx = FEN::tx_from_request(request(STORE_ADDRESS, 0, 7, chain_id)).unwrap();
    assert_eq!(tx.signer(), caller);
    assert!(evm.transact(&tx).unwrap().commit().status);
    assert_eq!(slot(&mut evm), U256::from(7));

    // A signed envelope round-trips through its exact encoding and recovers its sender.
    let mut unsigned = request(STORE_ADDRESS, 1, 8, chain_id).build_unsigned().unwrap();
    let signature = signer.sign_transaction_sync(&mut unsigned).unwrap();
    let envelope = TxEnvelopeFor::<FEN>::from(unsigned.into_signed(signature));
    assert_eq!(envelope.recover_signer().unwrap(), caller);
    let bytes = alloy_eips::Encodable2718::encoded_2718(&envelope);
    let tx = FEN::decode_tx(&bytes).unwrap();
    assert_eq!(tx.signer(), caller);
    assert!(evm.transact(&tx).unwrap().commit().status);
    assert_eq!(slot(&mut evm), U256::from(8));

    // A reverted transaction keeps none of its writes; a discarded one neither.
    let tx = FEN::tx_from_request(request(REVERT_ADDRESS, 2, 9, chain_id)).unwrap();
    assert!(!evm.transact(&tx).unwrap().commit().status);
    let tx = FEN::tx_from_request(request(STORE_ADDRESS, 3, 10, chain_id)).unwrap();
    assert!(evm.transact(&tx).unwrap().discard().status);
    assert_eq!(slot(&mut evm), U256::from(8));
    assert_eq!(nonce(&mut evm, caller), 3);

    // Snapshots restore state; removal keeps later snapshots.
    let first = lifecycle.snapshot(&mut evm);
    let tx = FEN::tx_from_request(request(STORE_ADDRESS, 3, 11, chain_id)).unwrap();
    assert!(evm.transact(&tx).unwrap().commit().status);
    let second = lifecycle.snapshot(&mut evm);
    let keep = RevertStateSnapshotAction::RevertKeep;
    let remove = RevertStateSnapshotAction::RevertRemove;
    let phase = ExecutionPhase::Boundary;
    assert!(lifecycle.revert(&mut evm, first, keep, phase).is_some());
    assert_eq!((slot(&mut evm), nonce(&mut evm, caller)), (U256::from(8), 3));
    assert!(lifecycle.revert(&mut evm, first, remove, phase).is_some());
    assert!(lifecycle.revert(&mut evm, first, remove, phase).is_none());
    assert!(lifecycle.delete_snapshot(second), "later snapshot kept");

    // A nested transaction sees in-flight writes and merges its changes as journaled writes of
    // the running transaction; a failed one only keeps its nonce increment and fee payment.
    let checkpoint = evm.state().checkpoint();
    evm.state_mut().storage_slot(&STORE_ADDRESS, U256::ZERO).unwrap().set(U256::from(50));
    let other = Address::with_last_byte(0xC1);
    let tx =
        FEN::tx_from_request(store_request::<FEN>(other, REVERT_ADDRESS, 0, 51, chain_id)).unwrap();
    let nested = lifecycle.prepare_nested(&mut evm, tx, None, None).execute(&mut evm, None, false);
    assert!(!lifecycle.store_executed(nested.unwrap()).status);
    assert_eq!((slot(&mut evm), nonce(&mut evm, other)), (U256::from(50), 1));
    let tx =
        FEN::tx_from_request(store_request::<FEN>(other, STORE_ADDRESS, 1, 52, chain_id)).unwrap();
    let nested = lifecycle.prepare_nested(&mut evm, tx, None, None).execute(&mut evm, None, false);
    assert!(lifecycle.store_executed(nested.unwrap()).status);
    assert_eq!((slot(&mut evm), nonce(&mut evm, other)), (U256::from(52), 2));
    let features = evm.version().features;
    evm.state_mut().rollback(checkpoint, features);
    assert_eq!((slot(&mut evm), nonce(&mut evm, other)), (U256::from(8), 0));
    evm.state_mut().clear_transaction_state();

    // Forks keep their own accepted state; persistent accounts follow every switch.
    let persistent = Address::with_last_byte(0xD1);
    lifecycle.forks_mut().add_persistent(persistent);
    evm.state_mut().account(&persistent).unwrap().set_balance(U256::from(5));
    evm.state_mut().commit_transaction();
    let fork_env = EvmEnv::new(spec, factory.version(spec, chain_id), Default::default());
    let info = |n: u64| ForkInfo::new(ForkId::new("http://fork", Some(n)), "http://fork", Some(n));
    let fork = lifecycle.create_fork(info(1), db_with(&[(caller, ETHER)], 100), fork_env);
    lifecycle.select_fork(&mut evm, Some(fork), phase).unwrap();
    assert_eq!(lifecycle.forks().active(), Some(fork));
    assert_eq!(slot(&mut evm), U256::from(100));
    assert_eq!(balance(&mut evm, persistent), U256::from(5));
    let tx = FEN::tx_from_request(request(STORE_ADDRESS, 0, 101, chain_id)).unwrap();
    assert!(evm.transact(&tx).unwrap().commit().status);
    evm.state_mut().account(&persistent).unwrap().set_balance(U256::from(6));
    evm.state_mut().commit_transaction();

    // Reverting a snapshot taken on the fork restores every fork's state and the selection.
    let on_fork = lifecycle.snapshot(&mut evm);
    lifecycle.select_fork(&mut evm, None, phase).unwrap();
    assert_eq!(slot(&mut evm), U256::from(8));
    assert_eq!(balance(&mut evm, persistent), U256::from(6));
    lifecycle.select_fork(&mut evm, Some(fork), phase).unwrap();
    assert_eq!(slot(&mut evm), U256::from(101));
    lifecycle.select_fork(&mut evm, None, phase).unwrap();
    assert!(lifecycle.revert(&mut evm, on_fork, remove, phase).is_some());
    assert_eq!(lifecycle.forks().active(), Some(fork));
    assert_eq!(slot(&mut evm), U256::from(101));
    assert!(lifecycle.select_fork(&mut evm, Some(U256::from(9)), phase).is_err());

    // Rolling drops the fork's own writes but keeps persistent accounts.
    let rolled = db_with(&[(caller, ETHER)], 200);
    lifecycle.roll_fork(&mut evm, None, rolled, fork_env, None, phase).unwrap();
    assert_eq!(slot(&mut evm), U256::from(200));
    assert_eq!(nonce(&mut evm, caller), 0);
    assert_eq!(balance(&mut evm, persistent), U256::from(6));

    // A chain ID change in flight is deferred to the next transaction boundary.
    let other_chain = EvmEnv::new(spec, factory.version(spec, 1), Default::default());
    let chain_fork = lifecycle.create_fork(info(2), db_with(&[(caller, ETHER)], 300), other_chain);
    lifecycle.select_fork(&mut evm, Some(chain_fork), ExecutionPhase::InFlight).unwrap();
    assert_eq!(lifecycle.pending_env().map(EvmEnv::chain_id), Some(1));
    assert_eq!((slot(&mut evm), evm.version().chain_id), (U256::from(300), chain_id));
    lifecycle.begin_transaction(&mut evm);
    assert_eq!(evm.version().chain_id, 1);
    assert!(lifecycle.pending_env().is_none());
}

#[test]
fn lifecycle() {
    lifecycle_suite::<EthEvmNetwork>(EthEvmFactory::default(), SpecId::PRAGUE);
    lifecycle_suite::<crate::evm::TempoEvmNetwork>(
        tempo_evm::TempoEvmFactory::default(),
        tempo_hardfork::TempoHardfork::default(),
    );
    #[cfg(feature = "optimism")]
    lifecycle_suite::<crate::evm::OpEvmNetwork>(op_evm2::OpEvmFactory, Default::default());
    #[cfg(feature = "monad")]
    lifecycle_suite::<crate::evm::MonadEvmNetwork>(monad_evm2::MonadEvmFactory, Default::default());
    #[cfg(feature = "base")]
    lifecycle_suite::<crate::evm::BaseEvmNetwork>(
        base_evm2::BaseEvmFactory::default(),
        base_evm2::BaseSpecId::new(base_evm2::BaseUpgrade::Beryl),
    );
}

#[test]
fn eth_signed_transactions_pay_effective_fees() {
    let signer = PrivateKeySigner::random();
    let (caller, to, coinbase) =
        (signer.address(), Address::with_last_byte(0xA2), Address::with_last_byte(0xA3));
    let factory = EthEvmFactory::default();
    let mut env =
        EvmEnv::new(SpecId::PRAGUE, factory.version(SpecId::PRAGUE, 1), Default::default());
    env.block.basefee = U256::from(5 * GWEI);
    env.block.beneficiary = coinbase;
    let mut evm = FoundryEvmFactory::create_evm(&factory, db_with(&[(caller, ETHER)], 0), env);

    let mut unsigned = alloy_rpc_types::TransactionRequest::default()
        .with_to(to)
        .with_value(U256::from(7))
        .with_nonce(0)
        .with_chain_id(1)
        .with_gas_limit(50_000)
        .with_max_fee_per_gas(10 * GWEI)
        .with_max_priority_fee_per_gas(2 * GWEI)
        .build_unsigned()
        .unwrap();
    let signature = signer.sign_transaction_sync(&mut unsigned).unwrap();
    let envelope = alloy_consensus::TxEnvelope::from(unsigned.into_signed(signature));
    let tx = EthEvmNetwork::tx_from_signed(envelope).unwrap();
    let result = evm.transact(&tx).unwrap().commit();

    assert!(result.status);
    assert_eq!(result.tx_gas_used(), 21_000);
    // Effective price is min(max fee, base fee + priority) = 7 gwei; the coinbase earns only the
    // 2 gwei priority fee, so swapped fee fields would change both balances.
    assert_eq!(balance(&mut evm, caller), U256::from(ETHER - 7 - 21_000 * 7 * GWEI));
    assert_eq!(balance(&mut evm, to), U256::from(7));
    assert_eq!(balance(&mut evm, coinbase), U256::from(21_000 * 2 * GWEI));
}

#[test]
fn requests_without_sender_or_fields_fail() {
    let request = store_request::<EthEvmNetwork>(Address::ZERO, STORE_ADDRESS, 0, 1, 1);
    let mut no_sender = request.clone();
    no_sender.from = None;
    assert!(EthEvmNetwork::tx_from_request(no_sender).is_err());
    let mut no_gas = request;
    no_gas.gas = None;
    assert!(EthEvmNetwork::tx_from_request(no_gas).is_err());
    assert!(EthEvmNetwork::decode_tx(&[0x02, 0xc0]).is_err());
}

#[test]
fn spec_changes_keep_foundry_policy() {
    let caller = Address::with_last_byte(0xA1);
    let factory = EthEvmFactory::default();
    let mut env =
        EvmEnv::new(SpecId::CANCUN, factory.version(SpecId::CANCUN, 31337), Default::default());
    env.set_feature(EvmFeatures::NONCE_CHECK, false);
    env.version.memory_limit = 1 << 20;
    let delegated = Address::with_last_byte(0xD7);
    let mut db = db_with(&[(caller, ETHER)], 0);
    let mut info = AccountInfo::default().with_code(Bytecode::new_eip7702(STORE_ADDRESS));
    db.insert_contract(&mut info);
    db.insert_account_info(&delegated, info);
    let mut evm = FoundryEvmFactory::create_evm(&factory, db, env);

    // Like REVM, an existing delegation resolves before Prague and runs in the delegating account.
    let request = store_request::<EthEvmNetwork>(caller, delegated, 0, 42, 31337);
    assert!(
        evm.transact(&EthEvmNetwork::tx_from_request(request).unwrap()).unwrap().commit().status
    );
    let stored = evm.state_mut().storage_slot_untracked(&delegated, &U256::ZERO).unwrap();
    assert_eq!(stored, U256::from(42));

    let env = EvmEnv::new(SpecId::PRAGUE, *evm.version(), *evm.block());
    factory.set_env(&mut evm, env);

    assert_eq!(evm.config_spec_id(), SpecId::PRAGUE);
    assert!(evm.feature(EvmFeatures::EIP7623), "fork features follow the new spec");
    assert!(!evm.feature(EvmFeatures::NONCE_CHECK), "disabled checks stay disabled");
    assert!(evm.feature(EvmFeatures::BALANCE_CHECK), "untouched checks stay enabled");
    assert_eq!(evm.version().chain_id, 31337);
    assert_eq!(evm.version().memory_limit, 1 << 20);

    // A cap disabled before the cap existed stays disabled at Osaka.
    let mut env = evm_env(&evm);
    env.version.tx_gas_limit_cap = u64::MAX;
    assert_eq!(factory.env_with_spec(env, SpecId::OSAKA).version.tx_gas_limit_cap, u64::MAX);
    assert_ne!(factory.version(SpecId::OSAKA, 31337).tx_gas_limit_cap, u64::MAX);

    let request = store_request::<EthEvmNetwork>(caller, STORE_ADDRESS, 9, 1, 31337);
    let result = evm.transact(&EthEvmNetwork::tx_from_request(request).unwrap()).unwrap().commit();
    assert!(result.status, "a wrong nonce executes with the nonce check off");
}

#[test]
fn tempo_factory_seeds_local_genesis() {
    let factory = tempo_evm::TempoEvmFactory::default();
    let spec = tempo_hardfork::TempoHardfork::default();
    let env = EvmEnv::new(spec, factory.version(spec, 31337), Default::default());
    let mut evm = FoundryEvmFactory::create_evm(&factory, InMemoryDB::default(), env);

    let code_hash = |evm: &mut Evm<'_, _>, address| {
        evm.read_account_info(&address).unwrap().map(|info| info.code_hash).unwrap_or_default()
    };
    for address in [crate::tempo::PATH_USD_ADDRESS, crate::tempo::TIP_FEE_MANAGER_ADDRESS] {
        let hash = code_hash(&mut evm, address);
        assert!(hash != B256::ZERO && hash != alloy_primitives::KECCAK256_EMPTY, "{address}");
    }
}

#[cfg(feature = "optimism")]
#[test]
fn op_deposits_mint_without_fees() {
    let (caller, to) = (Address::with_last_byte(0xA1), Address::with_last_byte(0xA2));
    let factory = op_evm2::OpEvmFactory;
    let spec = op_evm2::OpSpecId::default();
    let env = EvmEnv::new(spec, factory.version(spec, 10), Default::default());
    let mut evm = FoundryEvmFactory::create_evm(&factory, InMemoryDB::default(), env);

    let deposit = op_alloy_consensus::TxDeposit {
        from: caller,
        to: TxKind::Call(to),
        mint: 10,
        value: U256::from(3),
        gas_limit: 100_000,
        ..Default::default()
    };
    let envelope =
        op_alloy_consensus::OpTxEnvelope::Deposit(alloy_primitives::Sealed::new(deposit));
    let bytes = alloy_eips::Encodable2718::encoded_2718(&envelope);
    let tx = crate::evm::OpEvmNetwork::decode_tx(&bytes).unwrap();
    let result = evm.transact(&tx).unwrap().commit();

    assert!(result.status);
    assert!(!result.ext.is_failed_deposit());
    // The deposit mints to the unfunded caller, then transfers the value without charging fees.
    assert_eq!(balance(&mut evm, caller), U256::from(7));
    assert_eq!(balance(&mut evm, to), U256::from(3));
}

#[cfg(feature = "monad")]
#[test]
fn monad_charges_the_full_gas_limit() {
    let (caller, to) = (Address::with_last_byte(0xA1), Address::with_last_byte(0xA2));
    let factory = monad_evm2::MonadEvmFactory;
    let spec = monad_evm2::MonadHardfork::default();
    let mut env = EvmEnv::new(spec, factory.version(spec, 1), Default::default());
    env.block.basefee = U256::from(5 * GWEI);
    let mut evm = FoundryEvmFactory::create_evm(&factory, db_with(&[(caller, ETHER)], 0), env);

    let request = alloy_rpc_types::TransactionRequest::default()
        .with_from(caller)
        .with_to(to)
        .with_value(U256::from(7))
        .with_nonce(0)
        .with_chain_id(1)
        .with_gas_limit(50_000)
        .with_max_fee_per_gas(10 * GWEI)
        .with_max_priority_fee_per_gas(2 * GWEI);
    let tx = crate::evm::MonadEvmNetwork::tx_from_request(request).unwrap();
    evm.ext_mut().for_transaction(&tx);
    assert_eq!(evm.ext().chain.current_block_senders, vec![caller]);
    assert!(evm.transact(&tx).unwrap().commit().status);
    // Monad charges the whole 50k gas limit, not the 21k used.
    assert_eq!(balance(&mut evm, caller), U256::from(ETHER - 7 - 50_000 * 7 * GWEI));
}

#[cfg(feature = "monad")]
#[test]
fn monad_caps_configured_memory_limit() {
    let factory = monad_evm2::MonadEvmFactory;
    let spec = monad_evm2::MonadHardfork::MonadNine;
    let mut env = EvmEnv::new(spec, factory.version(spec, 1), Default::default());
    env.version.memory_limit = 128 * 1024 * 1024;
    let evm = FoundryEvmFactory::create_evm(&factory, db_with(&[], 0), env);

    assert_eq!(evm.version().memory_limit, monad_evm2::MONAD_MEMORY_LIMIT);
}

#[cfg(feature = "monad")]
#[test]
fn monad_block_cursor_excludes_synthetic_and_future_transactions() {
    let tx = |from: Address| {
        let request = alloy_rpc_types::TransactionRequest::default()
            .with_from(from)
            .with_to(Address::ZERO)
            .with_nonce(0)
            .with_chain_id(1)
            .with_gas_limit(21_000)
            .with_max_fee_per_gas(GWEI)
            .with_max_priority_fee_per_gas(GWEI);
        crate::evm::MonadEvmNetwork::tx_from_request(request).unwrap()
    };
    let [parent, replayed, future, synthetic, running] =
        [1, 2, 3, 4, 5].map(Address::with_last_byte);

    // A cursor before the second transaction of a block excludes the block's later ones.
    let context = BlockContext::<crate::evm::MonadEvmNetwork>::new(
        Vec::new(),
        vec![tx(parent)],
        vec![tx(replayed), tx(future)],
    );
    let mut cursor = monad_evm2::MonadEvmExt::default();
    context.clone().before_transaction(1).unwrap().prepare(&mut cursor, 1);
    assert_eq!(cursor.chain.current_block_senders, vec![replayed]);

    // Synthetic transactions are appended at the cursor without extending it.
    let mut ext = cursor.clone();
    ext.for_next_transaction(&tx(synthetic));
    ext.for_next_transaction(&tx(running));
    assert_eq!(ext.chain.current_block_senders, vec![replayed, running]);
    assert_eq!(ext.chain.current_tx_index, 1);
    assert!(!ext.chain.sender_can_dip(replayed, false));
    assert!(!ext.chain.sender_can_dip(parent, false));
    assert!(ext.chain.sender_can_dip(future, false));
    assert!(ext.chain.sender_can_dip(synthetic, false));

    // Adopting another cursor keeps the running transaction's own entry.
    let mut child = monad_evm2::MonadEvmExt::default();
    let mut context = context;
    context.advance_block();
    context.prepare(&mut child, 0);
    ext.adopt_cursor(&child);
    assert_eq!(ext.chain.current_block_senders, vec![running]);
    assert_eq!(ext.chain.current_tx_index, 0);
    assert!(!ext.chain.sender_can_dip(future, false));
    assert!(!ext.chain.sender_can_dip(parent, false));
    assert!(ext.chain.sender_can_dip(synthetic, false));
}

#[test]
fn fork_position_only_advances_to_the_immediate_transaction() {
    let parent = BlockNumHash::new(9, B256::with_last_byte(9));
    let block = BlockNumHash::new(10, B256::with_last_byte(10));
    let after_parent = ForkPosition::AfterBlock(parent);

    assert_eq!(
        after_parent.after_transaction(block, parent.hash, 0, 2),
        Some(ForkPosition::BeforeTransaction { block, index: 1 })
    );
    assert_eq!(after_parent.after_transaction(block, parent.hash, 1, 2), None);
    assert_eq!(after_parent.after_transaction(block, B256::ZERO, 0, 2), None);

    let before_last = ForkPosition::BeforeTransaction { block, index: 1 };
    assert_eq!(
        before_last.after_transaction(block, parent.hash, 1, 2),
        Some(ForkPosition::AfterBlock(block))
    );
    assert_eq!(before_last.after_transaction(block, parent.hash, 0, 2), None);
    assert_eq!(before_last.after_transaction(block, parent.hash, 1, 1), None);
}

#[cfg(feature = "base")]
#[test]
fn base_factory_plants_precompile_sentinels() {
    let factory = base_evm2::BaseEvmFactory::default();
    let spec = base_evm2::BaseSpecId::new(base_evm2::BaseUpgrade::Beryl);
    let env = EvmEnv::new(spec, factory.version(spec, 8453), Default::default());
    let mut evm = FoundryEvmFactory::create_evm(&factory, InMemoryDB::default(), env);

    let mut planted = 0;
    for address in foundry_evm_networks::BASE_CODE_SENTINEL_ADDRESSES {
        let info = evm.read_account_info(address).unwrap();
        let active = foundry_evm_networks::is_base_precompile_active_at(*address, spec.upgrade());
        let has_code = info.is_some_and(|info| info.code_hash != alloy_primitives::KECCAK256_EMPTY);
        assert_eq!(has_code, active, "{address}");
        planted += usize::from(has_code);
    }
    assert!(planted > 0);
}

/// An Ethereum EIP-1559 transaction as a fork RPC endpoint returns it.
fn eth_rpc_transaction(from: Address) -> AnyRpcTransaction {
    let tx = TxEip1559 {
        chain_id: 1,
        nonce: 42,
        gas_limit: 21001,
        to: TxKind::Call(Address::with_last_byte(0xBB)),
        value: U256::from(101),
        ..Default::default()
    };
    let signed =
        Signed::new_unchecked(tx, Signature::new(U256::ZERO, U256::ZERO, false), B256::ZERO);
    rpc_transaction(AnyTxEnvelope::Ethereum(signed.into()), from)
}

fn rpc_transaction(envelope: AnyTxEnvelope, from: Address) -> AnyRpcTransaction {
    AnyRpcTransaction::new(WithOtherFields::new(RpcTransaction {
        inner: Recovered::new_unchecked(envelope, from),
        block_hash: None,
        block_number: None,
        transaction_index: None,
        effective_gas_price: None,
        block_timestamp: None,
    }))
}

/// Every family converts Ethereum envelopes fetched over RPC and rejects unknown types.
fn rpc_conversion_suite<FEN: FoundryEvmNetwork>() {
    let from = Address::with_last_byte(0xAA);
    let tx = FEN::tx_from_any_rpc(&eth_rpc_transaction(from)).unwrap();
    assert_eq!((tx.signer(), tx.ty()), (from, 2));

    let unknown = AnyTxEnvelope::Unknown(UnknownTxEnvelope {
        hash: B256::ZERO,
        inner: UnknownTypedTransaction {
            ty: AnyTxType(0xFF),
            fields: Default::default(),
            memo: Default::default(),
        },
    });
    assert!(FEN::tx_from_any_rpc(&rpc_transaction(unknown, from)).is_err());
}

#[test]
fn from_any_rpc_transaction_for_eth() {
    let from = Address::random();
    let tx = EthEvmNetwork::tx_from_any_rpc(&eth_rpc_transaction(from)).unwrap();

    assert_eq!(tx.signer(), from);
    assert_eq!(tx.nonce(), 42);
    assert_eq!(tx.gas_limit(), 21001);
    assert_eq!(tx.value(), U256::from(101));
    assert_eq!(tx.kind(), TxKind::Call(Address::with_last_byte(0xBB)));
}

#[test]
fn rpc_transactions_convert_natively() {
    rpc_conversion_suite::<EthEvmNetwork>();
    rpc_conversion_suite::<crate::evm::TempoEvmNetwork>();
    #[cfg(feature = "optimism")]
    rpc_conversion_suite::<crate::evm::OpEvmNetwork>();
    #[cfg(feature = "monad")]
    rpc_conversion_suite::<crate::evm::MonadEvmNetwork>();
    #[cfg(feature = "base")]
    rpc_conversion_suite::<crate::evm::BaseEvmNetwork>();
}

#[test]
fn celo_fee_currency_transactions_execute_as_eip1559() {
    let mut json = serde_json::json!({
        "accessList": [],
        "blockHash": B256::ZERO,
        "blockNumber": "0x1",
        "chainId": "0x1",
        "feeCurrency": Address::with_last_byte(0xCC),
        "from": Address::with_last_byte(0xAA),
        "gas": "0x5208",
        "gasPrice": "0x3",
        "hash": B256::ZERO,
        "input": "0x1234",
        "maxFeePerGas": "0x3",
        "maxPriorityFeePerGas": "0x1",
        "nonce": "0x2a",
        "r": B256::ZERO,
        "s": B256::ZERO,
        "to": Address::with_last_byte(0xBB),
        "transactionIndex": "0x0",
        "type": "0x7b",
        "v": "0x0",
        "value": "0x65",
        "yParity": "0x0"
    });
    // Another network's type 0x7b envelope does not acquire CIP-64 semantics.
    let tx: AnyRpcTransaction = serde_json::from_value(json.clone()).unwrap();
    assert!(EthEvmNetwork::tx_from_any_rpc(&tx).is_err());

    json["chainId"] = "0xa4ec".into();
    let tx: AnyRpcTransaction = serde_json::from_value(json).unwrap();
    let tx = EthEvmNetwork::tx_from_any_rpc(&tx).unwrap();
    assert_eq!((tx.signer(), tx.ty(), tx.nonce()), (Address::with_last_byte(0xAA), 2, 42));
    assert_eq!((tx.max_fee_per_gas(), tx.max_priority_fee_per_gas()), (3, Some(1)));
    assert_eq!((tx.value(), tx.input().as_ref()), (U256::from(0x65), &[0x12, 0x34][..]));
    assert_eq!(tx.kind(), TxKind::Call(Address::with_last_byte(0xBB)));
}

#[test]
fn from_any_rpc_transaction_for_tempo_eth_envelope() {
    let from = Address::random();
    // evm2 hashes the re-encoded envelope, so the RPC hash must be the real one.
    let signed = eth_rpc_transaction(from).as_envelope().unwrap().as_eip1559().unwrap().clone();
    let signature = *signed.signature();
    let signed = Signed::new_unhashed(signed.strip_signature(), signature);
    let any_tx = rpc_transaction(AnyTxEnvelope::Ethereum(signed.into()), from);
    let tx = crate::evm::TempoEvmNetwork::tx_from_any_rpc(&any_tx).unwrap();

    assert_eq!(tx.signer(), from);
    assert_eq!(tx.nonce(), 42);
    assert_eq!(tx.gas_limit(), 21001);
    assert_eq!(tx.value(), U256::from(101));
    assert_eq!(tx.evm_tx().fee_token(), None);
    let tx_hash = any_tx.tx_hash();
    assert_eq!(tx.execution_context(), tempo_evm::ExecutionContext::Transaction { tx_hash });
    let envelope = crate::evm::TempoEvmNetwork::envelope_from_any_rpc(&any_tx).unwrap();
    assert_eq!(tx.unique_tx_identifier(), envelope.unique_tx_identifier(from));
    assert!(tx.as_aa().is_none());
}

#[test]
fn from_any_rpc_transaction_for_tempo_aa() {
    use alloy_signer::SignerSync;
    use std::num::NonZeroU64;
    use tempo_alloy::primitives::{TempoTransaction, TempoTxEnvelope, transaction::Call};

    let sender = PrivateKeySigner::random();
    let sponsor = PrivateKeySigner::random();
    let fee_token = Some(Address::random());
    let calls = vec![
        Call {
            to: TxKind::Call(Address::with_last_byte(0x11)),
            value: U256::ZERO,
            input: Bytes::from_static(&[0xaa, 0xbb]),
        },
        Call {
            to: TxKind::Call(Address::with_last_byte(0x22)),
            value: U256::ZERO,
            input: Bytes::from_static(&[0xcc, 0xdd]),
        },
    ];
    let mut tempo_tx = TempoTransaction {
        chain_id: 42431,
        nonce: 42,
        gas_limit: 424242,
        fee_token,
        nonce_key: U256::from(4242),
        valid_after: NonZeroU64::new(1800000000),
        valid_before: NonZeroU64::new(1900000000),
        calls,
        ..Default::default()
    };
    tempo_tx.fee_payer_signature =
        Some(sponsor.sign_hash_sync(&tempo_tx.fee_payer_signature_hash(sender.address())).unwrap());
    let signature = sender.sign_hash_sync(&tempo_tx.signature_hash()).unwrap();
    let aa_signed = tempo_tx.into_signed(signature.into());
    let tx_hash = *aa_signed.hash();
    let unique_tx_identifier = aa_signed.expiring_nonce_hash(sender.address());

    // Round-trip a Tempo RPC transaction through JSON, as `AnyNetwork` providers receive it.
    let rpc_tx = RpcTransaction::from_transaction(
        Recovered::new_unchecked(TempoTxEnvelope::AA(aa_signed), sender.address()),
        alloy_rpc_types::TransactionInfo::default(),
    );
    let any_tx: AnyRpcTransaction =
        serde_json::from_value(serde_json::to_value(&rpc_tx).unwrap()).unwrap();
    assert!((*any_tx.inner.inner).is_unknown());

    let tx = crate::evm::TempoEvmNetwork::tx_from_any_rpc(&any_tx).unwrap();
    assert_eq!(tx.ty(), 0x76);
    assert_eq!(tx.signer(), sender.address());
    assert_eq!(tx.nonce(), 42);
    assert_eq!(tx.gas_limit(), 424242);
    assert_eq!(tx.chain_id(), Some(42431));
    assert_eq!(tx.evm_tx().fee_token(), fee_token);
    assert_eq!(tx.fee_payer().unwrap(), sponsor.address());
    assert_eq!(tx.execution_context(), tempo_evm::ExecutionContext::Transaction { tx_hash });
    assert_eq!(tx.unique_tx_identifier(), unique_tx_identifier);

    let aa = tx.as_aa().unwrap().tx();
    assert_eq!(
        aa.calls.iter().map(|call| (call.to, call.input.clone())).collect::<Vec<_>>(),
        [
            (TxKind::Call(Address::with_last_byte(0x11)), Bytes::from_static(&[0xaa, 0xbb])),
            (TxKind::Call(Address::with_last_byte(0x22)), Bytes::from_static(&[0xcc, 0xdd])),
        ]
    );
    assert_eq!(aa.nonce_key, U256::from(4242));
    assert_eq!(aa.valid_after.map(NonZeroU64::get), Some(1800000000));
    assert_eq!(aa.valid_before.map(NonZeroU64::get), Some(1900000000));
    assert_eq!(tx.tx_hash(), tx_hash);
}

/// Decodes a captured Tempo `eth_getTransactionByHash` result.
fn captured_tempo_aa(json: &str) -> Recovered<tempo_evm::TempoTxEnv> {
    let any_tx: AnyRpcTransaction = serde_json::from_str(json).unwrap();
    let envelope = crate::evm::TempoEvmNetwork::envelope_from_any_rpc(&any_tx).unwrap();
    assert_eq!(alloy_primitives::keccak256(envelope.encoded_2718()), any_tx.tx_hash());
    let tx = crate::evm::TempoEvmNetwork::tx_from_any_rpc(&any_tx).unwrap();
    let aa = tx.as_aa().unwrap();
    // Recovery over the recomputed signing hash only succeeds if every signed field decoded.
    assert_eq!(aa.signature().recover_signer(&aa.signature_hash()).unwrap(), tx.signer());
    assert_eq!(tx.ty(), 0x76);
    tx
}

fn tempo_call(to: Address, input: Bytes) -> (TxKind, U256, Bytes) {
    (TxKind::Call(to), U256::ZERO, input)
}

fn tempo_calls(tx: &tempo_evm::TempoTxEnv) -> Vec<(TxKind, U256, Bytes)> {
    tx.as_aa()
        .unwrap()
        .tx()
        .calls
        .iter()
        .map(|call| (call.to, call.value, call.input.clone()))
        .collect()
}

#[test]
fn from_any_rpc_transaction_for_captured_sponsored_tempo_aa_batch() {
    use alloy_primitives::{address, b256, bytes};

    let tx = captured_tempo_aa(include_str!("../../test-data/tempo-aa-sponsored-batch.json"));
    assert_eq!(tx.signer(), address!("0x0a0d9bc4dda3a659699ce05153ffe9298f99ce89"));
    assert_eq!(tx.nonce(), 0);
    assert_eq!(tx.gas_limit(), 1403789);
    assert_eq!(tx.chain_id(), Some(4217));
    assert_eq!(
        tx.evm_tx().fee_token(),
        Some(address!("0x20c0000000000000000000006a37da5c996874be"))
    );
    assert_eq!(tx.fee_payer().unwrap(), address!("0x58aa7ce42e1d13b2919e2ac7e006c4fbc171442c"));
    assert_eq!(
        tx.execution_context(),
        tempo_evm::ExecutionContext::Transaction {
            tx_hash: b256!("0xbb23551c35bc2c2539ac3637c7464a22b5fb38bbb9cf39fa1f533ca6c2c981a3")
        }
    );
    // Expiring nonces are replay-protected by this sender-scoped identifier.
    assert_eq!(
        tx.channel_open_context_hash(),
        b256!("0xadb35ee9830a691a8dbd8f42208a1553d9a242b8750a218d841cc78a0dea20ca")
    );

    assert_eq!(
        tempo_calls(&tx),
        [
            tempo_call(
                address!("0x20c0000000000000000000000000000000000000"),
                bytes!(
                    "0x095ea7b300000000000000000000000083a1491f3e7f8daab8f787a631334b9ca7a870230000000000000000000000000000000000000000000000000000000077359400"
                ),
            ),
            tempo_call(
                address!("0x83a1491f3e7f8daab8f787a631334b9ca7a87023"),
                bytes!(
                    "0x6e553f6500000000000000000000000000000000000000000000000000000000773594000000000000000000000000000a0d9bc4dda3a659699ce05153ffe9298f99ce89"
                ),
            ),
        ]
    );
    let aa = tx.as_aa().unwrap().tx();
    assert_eq!(aa.nonce_key, U256::MAX);
    assert_eq!(aa.valid_after.map(|v| v.get()), Some(542204929));
    assert_eq!(aa.valid_before.map(|v| v.get()), Some(1790813692));
}

#[test]
fn from_any_rpc_transaction_for_captured_nonce_lane_tempo_aa() {
    use alloy_primitives::{address, b256};

    let tx = captured_tempo_aa(include_str!("../../test-data/tempo-aa-nonce-lane.json"));
    let sender = address!("0xfdd1c606b498f5fcaaf27bd318b14caf52e8f6c2");
    assert_eq!(tx.signer(), sender);
    assert_eq!(tx.nonce(), 22496);
    assert_eq!(tx.gas_limit(), 99408);
    assert_eq!(tx.chain_id(), Some(42431));
    assert_eq!(tx.evm_tx().fee_token(), None);
    // Without a sponsor the sender pays.
    assert_eq!(tx.fee_payer().unwrap(), sender);
    assert_eq!(
        tx.execution_context(),
        tempo_evm::ExecutionContext::Transaction {
            tx_hash: b256!("0x4df0a9b9e859be6c91c00edf8478939d0b91f6f9eb39dd9c8a93946ac70ff740")
        }
    );

    let calls = tempo_calls(&tx);
    let [(to, value, input)] = calls.as_slice() else { panic!("expected one call") };
    assert_eq!(*to, TxKind::Call(address!("0x5ad0000000000000000000000000000000000003")));
    assert_eq!(*value, U256::ZERO);
    assert_eq!(input.len(), 868);
    let aa = tx.as_aa().unwrap().tx();
    assert_eq!(aa.nonce_key, U256::ONE);
    assert_eq!(aa.valid_after, None);
    assert_eq!(aa.valid_before, None);
}

#[test]
fn from_any_rpc_transaction_for_captured_keychain_tempo_aa() {
    use alloy_primitives::{address, b256};
    use tempo_primitives::TempoSignature;

    let tx = captured_tempo_aa(include_str!("../../test-data/tempo-aa-keychain.json"));
    let sender = address!("0x5f704c6c7075acd14ee36527f03b5b5dcb4a966f");
    let key_id = address!("0x240a31713e851acdc0c590fe45ef72baeff55f91");
    assert_eq!(tx.signer(), sender);
    assert_eq!(tx.nonce(), 0);
    assert_eq!(tx.gas_limit(), 322821);
    assert_eq!(tx.chain_id(), Some(42431));
    assert_eq!(tx.evm_tx().fee_token(), None);
    assert_eq!(tx.fee_payer().unwrap(), address!("0x133d7736f290fa2758cf0d1e0862f5a93ae1dcbf"));
    assert_eq!(
        tx.execution_context(),
        tempo_evm::ExecutionContext::Transaction {
            tx_hash: b256!("0xd4d372ad5cfa4cdf476aeb25d4a76fb80fe2d6169eb9354c544500c0496a19d6")
        }
    );
    // Expiring nonces are replay-protected by this sender-scoped identifier.
    assert_eq!(
        tx.channel_open_context_hash(),
        b256!("0xdb2722c83d8893c1637bbabfe2ad924bcc044653f6c9fa91d472d70ddfa97de3")
    );

    assert_eq!(tempo_calls(&tx), [tempo_call(Address::ZERO, Bytes::new())]);
    let signed = tx.as_aa().unwrap();
    let aa = signed.tx();
    assert_eq!(aa.nonce_key, U256::MAX);
    assert_eq!(aa.valid_after.map(|v| v.get()), Some(949691873));
    assert_eq!(aa.valid_before.map(|v| v.get()), Some(1790811579));
    let TempoSignature::Keychain(keychain) = signed.signature() else {
        panic!("expected keychain")
    };
    assert_eq!(keychain.user_address, sender);
    assert_eq!(keychain.key_id(&signed.signature_hash()).unwrap(), key_id);
    let key_authorization = aa.key_authorization.as_ref().unwrap();
    assert_eq!(key_authorization.key_id, key_id);
    assert_eq!(key_authorization.recover_signer().unwrap(), sender);
}

#[test]
fn from_any_rpc_transaction_for_captured_authorization_list_tempo_aa() {
    use alloy_primitives::{address, b256};

    let tx = captured_tempo_aa(include_str!("../../test-data/tempo-aa-authorization-list.json"));
    let sender = address!("0xd410bcea80de9214e0e49df7d19c4c5e3db60f2a");
    assert_eq!(tx.signer(), sender);
    assert_eq!(tx.nonce(), 0);
    assert_eq!(tx.gas_limit(), 2000000);
    assert_eq!(tx.chain_id(), Some(42431));
    assert_eq!(
        tx.evm_tx().fee_token(),
        Some(address!("0x20c0000000000000000000000000000000000000"))
    );
    assert_eq!(tx.fee_payer().unwrap(), sender);
    assert_eq!(
        tx.execution_context(),
        tempo_evm::ExecutionContext::Transaction {
            tx_hash: b256!("0xfff5a6fa344fa9d6d7410374f40b6f711735500892cccdf480670fd7c5adcf93")
        }
    );

    let call = tempo_call(address!("0xa98d41b22c17ae4f4a94420a29a3095464c037e3"), Bytes::new());
    assert_eq!(tempo_calls(&tx), [call.clone(), call]);
    let aa = tx.as_aa().unwrap().tx();
    assert_eq!(aa.nonce_key, U256::ZERO);
    // Secp256k1, P256 and WebAuthn authorizations, in that order.
    let delegate = address!("0xaaaaaaaa00000000000000000000000000000000");
    assert_eq!(
        aa.tempo_authorization_list
            .iter()
            .map(|auth| (auth.address, auth.recover_authority().ok()))
            .collect::<Vec<_>>(),
        [
            (delegate, Some(address!("0xd9aa283bc5643587f9623a0e9683df78588404a8"))),
            (delegate, Some(address!("0x883b644ccaa219845187fd0932a8ef6e7d484bd6"))),
            (delegate, Some(address!("0x732c32ec8e029d8989b9d1db9914376b17e20676"))),
        ]
    );
}

#[test]
fn from_any_rpc_transaction_for_tempo_aa_without_signature_errors() {
    let mut json: serde_json::Value =
        serde_json::from_str(include_str!("../../test-data/tempo-aa-sponsored-batch.json"))
            .unwrap();
    json.as_object_mut().unwrap().remove("signature");
    let any_tx: AnyRpcTransaction = serde_json::from_value(json).unwrap();

    let err = crate::evm::TempoEvmNetwork::tx_from_any_rpc(&any_tx).unwrap_err();
    assert_eq!(err.to_string(), "cannot decode Tempo AA transaction: missing field `signature`");
}
