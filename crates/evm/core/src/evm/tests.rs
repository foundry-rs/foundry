use super::*;
use crate::{backend::RevertStateSnapshotAction, fork::ForkId};
use alloy_consensus::TxEip1559;
use alloy_eips::Encodable2718;
use alloy_network::{
    AnyTxEnvelope, AnyTxType, TxSignerSync, UnknownTxEnvelope, UnknownTypedTransaction,
};
use alloy_primitives::{B256, Bytes, TxKind, U256};
use alloy_rpc_types::Transaction as RpcTransaction;
use alloy_serde::WithOtherFields;
use alloy_signer_local::PrivateKeySigner;
use evm2::{
    EvmFeatures, SpecId,
    bytecode::Bytecode,
    evm::{AccountInfo, InMemoryDB},
};

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
    let mut lifecycle = ExecutionLifecycle::<_, &'static str>::new(
        factory.clone(),
        db_with(&[(caller, ETHER)], 1),
        env,
    );
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

    // Snapshots restore state and return their extra; removal also drops later snapshots.
    let first = lifecycle.snapshot(&mut evm, "first");
    let tx = FEN::tx_from_request(request(STORE_ADDRESS, 3, 11, chain_id)).unwrap();
    assert!(evm.transact(&tx).unwrap().commit().status);
    let second = lifecycle.snapshot(&mut evm, "second");
    let keep = RevertStateSnapshotAction::RevertKeep;
    let remove = RevertStateSnapshotAction::RevertRemove;
    let phase = ExecutionPhase::Boundary;
    let (extra, transition) = lifecycle.revert(&mut evm, first, keep, phase).unwrap();
    assert_eq!((extra, transition), ("first", EnvTransition::Applied));
    assert_eq!((slot(&mut evm), nonce(&mut evm, caller)), (U256::from(8), 3));
    let (extra, _) = lifecycle.revert(&mut evm, first, remove, phase).unwrap();
    assert_eq!(extra, "first");
    assert!(lifecycle.revert(&mut evm, first, remove, phase).is_none());
    assert!(lifecycle.revert(&mut evm, second, remove, phase).is_none(), "later snapshot dropped");

    // A nested frame rolls back only when it fails.
    let failed = nested_frame(&mut evm, |evm| {
        evm.state_mut().storage_slot(&STORE_ADDRESS, U256::ZERO).unwrap().set(U256::from(99));
        Err::<(), _>("boom")
    });
    assert!(failed.is_err());
    assert_eq!(slot(&mut evm), U256::from(8));
    nested_frame(&mut evm, |evm| {
        evm.state_mut().storage_slot(&STORE_ADDRESS, U256::ZERO).unwrap().set(U256::from(98));
        Ok::<_, ()>(())
    })
    .unwrap();
    assert_eq!(slot(&mut evm), U256::from(98));
    evm.state_mut().clear_transaction_state();
    assert_eq!(slot(&mut evm), U256::from(8));

    // A nested transaction sees in-flight writes and merges its own only on success, as
    // journaled writes of the running transaction.
    let checkpoint = evm.state().checkpoint();
    evm.state_mut().storage_slot(&STORE_ADDRESS, U256::ZERO).unwrap().set(U256::from(50));
    let other = Address::with_last_byte(0xC1);
    let tx =
        FEN::tx_from_request(store_request::<FEN>(other, REVERT_ADDRESS, 0, 51, chain_id)).unwrap();
    assert!(!lifecycle.nested_transaction(&mut evm, &tx).unwrap().status);
    assert_eq!((slot(&mut evm), nonce(&mut evm, other)), (U256::from(50), 0));
    let tx =
        FEN::tx_from_request(store_request::<FEN>(other, STORE_ADDRESS, 0, 52, chain_id)).unwrap();
    assert!(lifecycle.nested_transaction(&mut evm, &tx).unwrap().status);
    assert_eq!((slot(&mut evm), nonce(&mut evm, other)), (U256::from(52), 1));
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
    assert_eq!(lifecycle.select_fork(&mut evm, Some(fork), phase).unwrap(), EnvTransition::Applied);
    assert_eq!(lifecycle.forks().active(), Some(fork));
    assert_eq!(slot(&mut evm), U256::from(100));
    assert_eq!(balance(&mut evm, persistent), U256::from(5));
    let tx = FEN::tx_from_request(request(STORE_ADDRESS, 0, 101, chain_id)).unwrap();
    assert!(evm.transact(&tx).unwrap().commit().status);
    evm.state_mut().account(&persistent).unwrap().set_balance(U256::from(6));
    evm.state_mut().commit_transaction();

    // Reverting a snapshot taken on the fork restores every fork's state and the selection.
    let on_fork = lifecycle.snapshot(&mut evm, "fork");
    assert_eq!(lifecycle.select_fork(&mut evm, None, phase).unwrap(), EnvTransition::Applied);
    assert_eq!(slot(&mut evm), U256::from(8));
    assert_eq!(balance(&mut evm, persistent), U256::from(6));
    assert_eq!(lifecycle.select_fork(&mut evm, Some(fork), phase).unwrap(), EnvTransition::Applied);
    assert_eq!(slot(&mut evm), U256::from(101));
    assert_eq!(lifecycle.select_fork(&mut evm, None, phase).unwrap(), EnvTransition::Applied);
    assert_eq!(
        lifecycle.revert(&mut evm, on_fork, remove, phase).unwrap().1,
        EnvTransition::Applied
    );
    assert_eq!(lifecycle.forks().active(), Some(fork));
    assert_eq!(slot(&mut evm), U256::from(101));
    assert!(lifecycle.select_fork(&mut evm, Some(U256::from(9)), phase).is_err());

    // Rolling drops the fork's own writes but keeps persistent accounts.
    let rolled = db_with(&[(caller, ETHER)], 200);
    let transition = lifecycle.roll_fork(&mut evm, None, rolled, fork_env, phase).unwrap();
    assert_eq!(transition, EnvTransition::Applied);
    assert_eq!(slot(&mut evm), U256::from(200));
    assert_eq!(nonce(&mut evm, caller), 0);
    assert_eq!(balance(&mut evm, persistent), U256::from(6));

    // A chain ID change in flight is deferred to the next transaction boundary.
    let other_chain = EvmEnv::new(spec, factory.version(spec, 1), Default::default());
    let chain_fork = lifecycle.create_fork(info(2), db_with(&[(caller, ETHER)], 300), other_chain);
    let transition =
        lifecycle.select_fork(&mut evm, Some(chain_fork), ExecutionPhase::InFlight).unwrap();
    assert_eq!(transition, EnvTransition::Deferred);
    assert_eq!((slot(&mut evm), evm.version().chain_id), (U256::from(300), chain_id));
    lifecycle.begin_transaction(&mut evm);
    assert_eq!(evm.version().chain_id, 1);
}

#[test]
fn eth_lifecycle() {
    lifecycle_suite::<EthEvmNetwork>(EthEvmFactory, SpecId::PRAGUE);
}

#[test]
fn tempo_lifecycle() {
    lifecycle_suite::<crate::evm::TempoEvmNetwork>(
        tempo_evm::TempoEvmFactory::default(),
        tempo_hardfork::TempoHardfork::default(),
    );
}

#[cfg(feature = "optimism")]
#[test]
fn op_lifecycle() {
    lifecycle_suite::<crate::evm::OpEvmNetwork>(
        op_evm2::OpEvmFactory,
        op_evm2::OpSpecId::default(),
    );
}

#[cfg(feature = "monad")]
#[test]
fn monad_lifecycle() {
    lifecycle_suite::<crate::evm::MonadEvmNetwork>(
        monad_evm2::MonadEvmFactory,
        monad_evm2::MonadHardfork::default(),
    );
}

#[cfg(feature = "base")]
#[test]
fn base_lifecycle() {
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
    let factory = EthEvmFactory;
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
    let factory = EthEvmFactory;
    let mut env =
        EvmEnv::new(SpecId::CANCUN, factory.version(SpecId::CANCUN, 31337), Default::default());
    env.set_feature(EvmFeatures::NONCE_CHECK, false);
    env.version.memory_limit = 1 << 20;
    let mut evm = FoundryEvmFactory::create_evm(&factory, db_with(&[(caller, ETHER)], 0), env);

    factory.set_execution_spec(&mut evm, SpecId::PRAGUE);

    assert_eq!(evm.config_spec_id(), SpecId::PRAGUE);
    assert!(evm.feature(EvmFeatures::EIP7702), "fork features follow the new spec");
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
fn celo_fee_currency_transactions_fail_explicitly() {
    let tx: AnyRpcTransaction = serde_json::from_value(serde_json::json!({
        "accessList": [],
        "blockHash": B256::ZERO,
        "blockNumber": "0x1",
        "chainId": "0xa4ec",
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
    }))
    .unwrap();
    assert_eq!(
        EthEvmNetwork::tx_from_any_rpc(&tx).unwrap_err().to_string(),
        format!(
            "cannot replay Celo CIP-64 transaction {}: evm2 has no fee-currency transaction type",
            B256::ZERO
        )
    );
}

#[test]
fn captured_tempo_aa_rpc_transactions_convert_exactly() {
    for json in [
        include_str!("../../test-data/tempo-aa-sponsored-batch.json"),
        include_str!("../../test-data/tempo-aa-nonce-lane.json"),
        include_str!("../../test-data/tempo-aa-keychain.json"),
        include_str!("../../test-data/tempo-aa-authorization-list.json"),
    ] {
        let rpc: AnyRpcTransaction = serde_json::from_str(json).unwrap();
        let envelope = crate::evm::TempoEvmNetwork::envelope_from_any_rpc(&rpc).unwrap();
        // The hash and the recovered signer only match if every signed field decoded.
        assert_eq!(alloy_primitives::keccak256(envelope.encoded_2718()), rpc.tx_hash());
        assert_eq!(envelope.recover_signer().unwrap(), rpc.from());
        let tx = crate::evm::TempoEvmNetwork::tx_from_any_rpc(&rpc).unwrap();
        assert_eq!((tx.signer(), tx.ty()), (rpc.from(), 0x76));
    }
}
