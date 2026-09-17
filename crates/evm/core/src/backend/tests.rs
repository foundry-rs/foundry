//! Tests for backend fork state, replay, and cache behavior.

use super::{Fork, ForkAccountField, ReplayInputs, apply_state_changeset, update_env_block};
use crate::{
    backend::{Backend, DatabaseExt, ForkPosition},
    evm::EthEvmNetwork,
    fork::{CreateFork, ForkId, MultiFork},
    opts::EvmOpts,
};
use alloy_consensus::{Signed, TxEnvelope, TxLegacy, transaction::Recovered};
use alloy_eips::BlockNumHash;
use alloy_evm::EvmEnv;
use alloy_network::{
    AnyHeader, AnyNetwork, AnyRpcBlock, AnyRpcHeader, AnyRpcTransaction, AnyTxEnvelope, AnyTxType,
    TransactionBuilder, UnknownTxEnvelope, UnknownTypedTransaction,
};
use alloy_primitives::{
    Address, B256, Bytes, Signature, TxKind, U256, address, keccak256, map::AddressSet,
};
use alloy_provider::{Provider, ProviderBuilder, mock::Asserter};
use alloy_rpc_types::{
    Block, BlockTransactions, Transaction as RpcTransaction, TransactionRequest,
};
use alloy_serde::WithOtherFields;
use alloy_sol_types::SolValue;
use anvil::{NodeConfig, spawn};
use foundry_common::{SYSTEM_TRANSACTION_TYPE, provider::get_http_provider};
use foundry_config::{Config, NamedChain};
use foundry_evm_networks::{NetworkConfigs, celo::transfer::CELO_TRANSFER_ADDRESS};
use foundry_fork_db::{
    SharedBackend,
    cache::{BlockchainDb, BlockchainDbMeta},
};
use revm::{
    context::{BlockEnv, JournalInner, TxEnv},
    database::{AccountState, CacheDB, DatabaseRef, DbAccount},
    primitives::{KECCAK_EMPTY, hardfork::SpecId},
    state::{Account, AccountInfo, EvmState, EvmStorageSlot, TransactionId},
};

#[cfg(feature = "monad")]
use super::ensure_block_identity;
#[cfg(feature = "monad")]
use crate::evm::monad::BlockContext;
#[cfg(feature = "monad")]
use monad_revm::{
    MonadHardfork,
    api::block::syscall_snapshot_calldata,
    staking::{STAKING_ADDRESS, constants::SYSTEM_ADDRESS},
};

fn fork_with_closed_backend() -> Fork<AnyNetwork, BlockEnv> {
    let provider =
        ProviderBuilder::<_, _, AnyNetwork>::default().connect_mocked_client(Asserter::new());
    let db = BlockchainDb::new(
        BlockchainDbMeta::new(BlockEnv::default(), "http://localhost".to_string()),
        None,
    );
    let (backend, handler) = SharedBackend::new(provider, db, None);
    drop(handler);
    Fork {
        db: CacheDB::new(backend),
        journaled_state: JournalInner::new(),
        source_chain_id: 1,
        position: ForkPosition::AfterBlock { block: BlockNumHash::default() },
    }
}

fn rpc_block(number: u64, hash: B256, parent_hash: B256) -> AnyRpcBlock {
    let header = AnyHeader { number, parent_hash, ..Default::default() };
    AnyRpcBlock::new(
        Block::new(
            AnyRpcHeader::from_sealed(header.seal(hash)),
            BlockTransactions::Full(Vec::new()),
        )
        .into(),
    )
}

fn rpc_transaction(
    caller: Address,
    nonce: u64,
    value: u64,
    gas_limit: u64,
    recipient: Address,
    hash: B256,
) -> AnyRpcTransaction {
    let tx = TxLegacy {
        nonce,
        gas_limit,
        to: TxKind::Call(recipient),
        value: U256::from(value),
        ..Default::default()
    };
    let signed =
        Signed::new_unchecked(tx, Signature::new(U256::from(1), U256::from(1), false), hash);
    AnyRpcTransaction::new(WithOtherFields::new(RpcTransaction {
        inner: Recovered::new_unchecked(
            AnyTxEnvelope::Ethereum(TxEnvelope::Legacy(signed)),
            caller,
        ),
        block_hash: None,
        block_number: None,
        transaction_index: None,
        effective_gas_price: None,
        block_timestamp: None,
    }))
}

#[test]
fn fork_replay_backend_reuses_manager_on_current_thread_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

    let forks = runtime.block_on(async {
        let forks = MultiFork::<AnyNetwork, SpecId, BlockEnv>::spawn();
        let sender = Address::with_last_byte(0x42);
        let recipient = Address::with_last_byte(0x43);
        let target = B256::with_last_byte(2);
        let mut fork = fork_with_closed_backend();
        fork.db.insert_account_info(
            sender,
            AccountInfo { balance: U256::from(100), ..Default::default() },
        );
        fork.db.insert_account_info(recipient, AccountInfo::default());
        fork.db.insert_account_info(Address::ZERO, AccountInfo::default());
        let mut block = rpc_block(1, B256::with_last_byte(1), B256::ZERO);
        block.inner.transactions = BlockTransactions::Full(vec![
            rpc_transaction(sender, 0, 7, 21_000, recipient, B256::with_last_byte(1)),
            rpc_transaction(sender, 1, 0, 21_000, recipient, target),
        ]);

        let result = Backend::<EthEvmNetwork>::replay_until(
            &mut fork,
            ReplayInputs {
                fork_id: ForkId::new("http://localhost", Some(0)),
                forks: forks.clone(),
                evm_env: EvmEnv::default(),
                networks: NetworkConfigs::default(),
            },
            &block,
            #[cfg(feature = "monad")]
            None,
            target,
            &mut JournalInner::new(),
            &AddressSet::default(),
        )
        .unwrap();

        assert!(result.is_some());
        assert_eq!(fork.db.basic_ref(recipient).unwrap().unwrap().balance, U256::from(7));
        assert_eq!(fork.db.basic_ref(sender).unwrap().unwrap().nonce, 1);
        forks
    });
    drop(runtime);
    drop(forks);
}

#[test]
fn fork_replay_keeps_system_skips_and_prefix_atomicity() {
    let sender = Address::with_last_byte(0x42);
    let recipient = Address::with_last_byte(0x43);
    let system = address!("6f49a8f621353f12378d0046e7d7e4b9b249dc9e");
    let target = B256::with_last_byte(4);

    #[cfg(not(feature = "monad"))]
    let contexts = [false];
    #[cfg(feature = "monad")]
    let contexts = [false, true];
    for _with_context in contexts {
        for invalid_nonce in [false, true] {
            let mut fork = fork_with_closed_backend();
            fork.db.insert_account_info(
                sender,
                AccountInfo { balance: U256::from(100), ..Default::default() },
            );
            fork.db.insert_account_info(recipient, AccountInfo::default());
            fork.db.insert_account_info(Address::ZERO, AccountInfo::default());
            let block = AnyRpcBlock::new(
                Block::new(
                    AnyRpcHeader::from_sealed(
                        AnyHeader { number: 1, ..Default::default() }.seal(B256::with_last_byte(1)),
                    ),
                    BlockTransactions::Full(vec![
                        rpc_transaction(sender, 0, 7, 21_000, recipient, B256::with_last_byte(1)),
                        // Ethereum must skip this envelope, not validate it as an ordinary
                        // call.
                        rpc_transaction(system, 0, 0, 0, recipient, B256::with_last_byte(2)),
                        rpc_transaction(
                            sender,
                            if invalid_nonce { 0 } else { 1 },
                            5,
                            21_000,
                            recipient,
                            B256::with_last_byte(3),
                        ),
                        rpc_transaction(sender, 2, 2, 21_000, recipient, target),
                    ]),
                )
                .into(),
            );
            let networks = NetworkConfigs::default();
            #[cfg(feature = "monad")]
            let networks = if _with_context { NetworkConfigs::with_monad() } else { networks };
            #[cfg(feature = "monad")]
            let context = _with_context
                .then(|| BlockContext::<EthEvmNetwork>::new(Vec::new(), Vec::new(), Vec::new()));
            let result = Backend::<EthEvmNetwork>::replay_until(
                &mut fork,
                ReplayInputs {
                    fork_id: ForkId::new("http://localhost", Some(0)),
                    forks: MultiFork::spawn(),
                    evm_env: EvmEnv::default(),
                    networks,
                },
                &block,
                #[cfg(feature = "monad")]
                context.as_ref(),
                target,
                &mut JournalInner::new(),
                &AddressSet::default(),
            );
            if invalid_nonce {
                assert!(result.is_err());
                assert_eq!(fork.db.basic_ref(recipient).unwrap().unwrap().balance, U256::ZERO);
                assert_eq!(fork.db.basic_ref(sender).unwrap().unwrap().nonce, 0);
            } else {
                assert!(result.unwrap().is_some());
                assert_eq!(fork.db.basic_ref(recipient).unwrap().unwrap().balance, U256::from(12));
                assert_eq!(fork.db.basic_ref(sender).unwrap().unwrap().nonce, 2);
            }
        }
    }
}

#[test]
#[cfg(feature = "monad")]
fn fork_replay_executes_monad_system_prefix_with_or_without_profile() {
    let transaction = |nonce, hash| {
        let tx = TxLegacy {
            nonce,
            to: TxKind::Call(STAKING_ADDRESS),
            input: syscall_snapshot_calldata(),
            ..Default::default()
        };
        let signed =
            Signed::new_unchecked(tx, Signature::new(U256::from(1), U256::from(1), false), hash);
        AnyRpcTransaction::new(WithOtherFields::new(RpcTransaction {
            inner: Recovered::new_unchecked(
                AnyTxEnvelope::Ethereum(TxEnvelope::Legacy(signed)),
                SYSTEM_ADDRESS,
            ),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            effective_gas_price: None,
            block_timestamp: None,
        }))
    };
    let target = B256::with_last_byte(2);
    let mut block = rpc_block(1, B256::with_last_byte(1), B256::ZERO);
    block.inner.transactions = BlockTransactions::Full(vec![
        transaction(3, B256::with_last_byte(1)),
        transaction(4, target),
    ]);
    for with_context in [false, true] {
        let mut fork = fork_with_closed_backend();
        fork.db.insert_account_info(SYSTEM_ADDRESS, AccountInfo { nonce: 3, ..Default::default() });
        fork.db.insert_account_info(STAKING_ADDRESS, AccountInfo::default());
        fork.db.replace_account_storage(STAKING_ADDRESS, Default::default()).unwrap();
        let context = with_context.then(|| {
            BlockContext::<crate::evm::MonadEvmNetwork>::new(Vec::new(), Vec::new(), Vec::new())
        });
        let result = Backend::<crate::evm::MonadEvmNetwork>::replay_until(
            &mut fork,
            ReplayInputs {
                fork_id: ForkId::new("http://localhost", Some(0)),
                forks: MultiFork::spawn(),
                evm_env: EvmEnv::new(
                    revm::context::CfgEnv::new_with_spec(MonadHardfork::MonadNine),
                    BlockEnv::default(),
                ),
                networks: if with_context {
                    NetworkConfigs::with_monad()
                } else {
                    NetworkConfigs::default()
                },
            },
            &block,
            context.as_ref(),
            target,
            &mut JournalInner::new(),
            &AddressSet::default(),
        );
        assert!(result.unwrap().is_some());
        assert_eq!(fork.db.basic_ref(SYSTEM_ADDRESS).unwrap().unwrap().nonce, 4);
    }
}

#[test]
#[cfg(feature = "monad")]
fn validates_block_identity() {
    let hash = B256::with_last_byte(2);
    let block = rpc_block(2, hash, B256::with_last_byte(1));
    assert!(ensure_block_identity(&block, BlockNumHash::new(2, hash), "parent").is_ok());

    let err =
        ensure_block_identity(&block, BlockNumHash::new(2, B256::with_last_byte(3)), "parent")
            .unwrap_err();
    assert!(err.to_string().contains("parent block changed"));

    let err = ensure_block_identity(&block, BlockNumHash::new(1, hash), "grandparent").unwrap_err();
    assert!(err.to_string().contains("grandparent block changed"));
}

#[test]
fn failed_fork_state_refresh_does_not_publish_transaction_changes() {
    let mut fork = fork_with_closed_backend();
    let externally_loaded = Address::with_last_byte(1);
    let fork_loaded = Address::with_last_byte(2);
    let committed = Address::with_last_byte(3);
    let missing_slot = U256::from(1);

    let cached_external = AccountInfo { balance: U256::from(11), ..Default::default() };
    let cached_fork = AccountInfo { balance: U256::from(12), ..Default::default() };
    fork.db.insert_account_info(externally_loaded, cached_external);
    fork.db.insert_account_info(fork_loaded, cached_fork);

    let mut journaled_state = JournalInner::new();
    let external_account =
        Account::default().with_info(AccountInfo { balance: U256::from(1), ..Default::default() });
    journaled_state.state.insert(externally_loaded, external_account);

    let mut fork_account =
        Account::default().with_info(AccountInfo { balance: U256::from(2), ..Default::default() });
    fork_account.storage.insert(missing_slot, EvmStorageSlot::new(U256::ZERO, TransactionId::ZERO));
    fork.journaled_state.state.insert(fork_loaded, fork_account);

    let mut committed_account =
        Account::default().with_info(AccountInfo { balance: U256::from(13), ..Default::default() });
    committed_account.mark_touch();
    let mut state = EvmState::default();
    state.insert(committed, committed_account);

    let result =
        apply_state_changeset(state, &mut journaled_state, &mut fork, &AddressSet::default());
    assert!(result.is_err());
    assert!(!fork.db.cache.accounts.contains_key(&committed));
    assert_eq!(journaled_state.state[&externally_loaded].info.balance, U256::from(1));
    assert_eq!(fork.journaled_state.state[&fork_loaded].info.balance, U256::from(2));
}

#[test]
fn failed_fork_state_refresh_preserves_not_existing_account() {
    let mut fork = fork_with_closed_backend();
    let address = Address::with_last_byte(1);
    let missing_slot = U256::from(1);
    fork.db.cache.accounts.insert(address, DbAccount::new_not_existing());

    let mut journaled_state = JournalInner::new();
    let mut journaled_account =
        Account::default().with_info(AccountInfo { balance: U256::from(1), ..Default::default() });
    journaled_account
        .storage
        .insert(missing_slot, EvmStorageSlot::new(U256::from(7), TransactionId::ZERO));
    journaled_state.state.insert(address, journaled_account);

    let mut touched_account =
        Account::default().with_info(AccountInfo { balance: U256::from(13), ..Default::default() });
    touched_account.mark_touch();
    let mut state = EvmState::default();
    state.insert(address, touched_account);

    let result =
        apply_state_changeset(state, &mut journaled_state, &mut fork, &AddressSet::default());
    assert!(result.is_err());
    assert_eq!(fork.db.cache.accounts[&address].account_state, AccountState::NotExisting);
    assert_eq!(journaled_state.state[&address].info.balance, U256::from(1));
    assert_eq!(
        journaled_state.state[&address].storage[&missing_slot].present_value(),
        U256::from(7)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn refresh_fork_account_updates_loaded_journals() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let target = address!("0x0000000000000000000000000000000000001331");
    let code = Bytes::from_static(&[0x60, 0x2a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);

    let provider = handle.http_provider();
    let block_number = provider.get_block_number().await.unwrap();
    let mut evm_opts = Config::figment().extract::<EvmOpts>().unwrap();
    evm_opts.fork_url = Some(handle.http_endpoint());
    evm_opts.fork_block_number = Some(block_number);
    let fork = evm_opts.get_fork(&Config::default(), 31_337, Some(block_number)).unwrap();
    let mut backend = Backend::<EthEvmNetwork>::spawn(Some(fork)).unwrap();

    let mut journaled_state = JournalInner::new();
    journaled_state.load_account(&mut backend, target).unwrap();
    journaled_state.state.get_mut(&target).unwrap().info.balance = U256::from(1);
    let fork = backend.active_fork_mut().unwrap();
    fork.journaled_state.load_account(&mut fork.db, target).unwrap();
    fork.journaled_state.state.get_mut(&target).unwrap().info.balance = U256::from(2);
    let cached = fork.db.cache.accounts.get_mut(&target).unwrap();
    cached.info.balance = U256::from(3);
    cached.account_state = AccountState::Touched;
    assert_eq!(journaled_state.state[&target].info.code_hash, KECCAK_EMPTY);
    assert_eq!(fork.journaled_state.state[&target].info.code_hash, KECCAK_EMPTY);

    api.anvil_set_code(target, code.clone()).await.unwrap();
    backend.refresh_fork_account(target, ForkAccountField::Code, &mut journaled_state).unwrap();

    let expected_hash = keccak256(&code);
    let refreshed = &journaled_state.state[&target].info;
    assert_eq!(refreshed.code_hash, expected_hash);
    assert_eq!(refreshed.code.as_ref().unwrap().original_bytes(), code);
    assert_eq!(refreshed.balance, U256::from(1));
    let refreshed = &backend.active_fork().unwrap().journaled_state.state[&target].info;
    assert_eq!(refreshed.code_hash, expected_hash);
    assert_eq!(refreshed.code.as_ref().unwrap().original_bytes(), code);
    assert_eq!(refreshed.balance, U256::from(2));
    let cached = &backend.active_fork().unwrap().db.cache.accounts[&target];
    assert_eq!(cached.info.code_hash, expected_hash);
    assert_eq!(cached.info.balance, U256::from(3));
}

#[test]
fn ethereum_replay_skips_unknown_system_envelopes_before_conversion() {
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

    assert!(
        Backend::<EthEvmNetwork>::replay_tx_env(&transaction(SYSTEM_TRANSACTION_TYPE))
            .unwrap()
            .is_none()
    );
    assert!(Backend::<EthEvmNetwork>::replay_tx_env(&transaction(0xff)).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn celo_transaction_hash_fork_replays_transfer_precompile() {
    let networks = NetworkConfigs::with_celo();
    let (api, handle) = spawn(
        NodeConfig::test().with_chain_id(Some(NamedChain::Celo as u64)).with_networks(networks),
    )
    .await;
    let provider = handle.http_provider();
    let sender = provider.get_accounts().await.unwrap()[0];
    let recipient = Address::with_last_byte(0x99);
    let transfer_amount = U256::from(1_000);
    let target_amount = U256::from(1);
    let nonce = provider.get_transaction_count(sender).await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();

    api.anvil_set_auto_mine(false).await.unwrap();
    api.send_transaction(WithOtherFields::new(
        TransactionRequest::default()
            .with_from(sender)
            .with_to(CELO_TRANSFER_ADDRESS)
            .with_nonce(nonce)
            .with_gas_limit(100_000)
            .with_gas_price(gas_price)
            .with_input(Bytes::from((sender, recipient, transfer_amount).abi_encode())),
    ))
    .await
    .unwrap();
    let target_hash = api
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .with_from(sender)
                .with_to(recipient)
                .with_nonce(nonce + 1)
                .with_gas_limit(21_000)
                .with_gas_price(gas_price)
                .with_value(target_amount),
        ))
        .await
        .unwrap();
    api.mine_one().await.unwrap();

    assert_eq!(provider.get_balance(recipient).await.unwrap(), transfer_amount + target_amount);

    let endpoint = handle.http_endpoint();
    let fork_block_number = provider.get_block_number().await.unwrap();
    let evm_opts = EvmOpts {
        fork_url: Some(endpoint.clone()),
        fork_block_number: Some(fork_block_number),
        networks,
        ..Default::default()
    };
    let fork = CreateFork { url: endpoint, enable_caching: false, evm_opts, resolved: None };
    let mut backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
    backend.set_networks(networks);

    let fork_id = backend.create_fork_at_transaction(fork, target_hash).unwrap();
    let fork = backend.inner.get_fork_by_id(fork_id).unwrap();
    assert!(matches!(fork.position, ForkPosition::BeforeTransaction { transaction_index: 1, .. }));
    assert_eq!(fork.db.basic_ref(recipient).unwrap().unwrap_or_default().balance, transfer_amount);
}

#[test]
fn fork_position_advances_from_exact_transaction_predecessor() {
    let parent_block = BlockNumHash::new(10, B256::with_last_byte(10));
    let block = BlockNumHash::new(11, B256::with_last_byte(11));
    let parent = ForkPosition::AfterBlock { block: parent_block };
    assert_eq!(
        parent.after_transaction(block, parent_block.hash, 0, 2),
        Some(ForkPosition::BeforeTransaction { block, transaction_index: 1 })
    );
    assert_eq!(
        parent.after_transaction(block, parent_block.hash, 0, 1),
        Some(ForkPosition::AfterBlock { block })
    );

    let before_first = ForkPosition::BeforeTransaction { block, transaction_index: 0 };
    assert_eq!(
        before_first.after_transaction(block, parent_block.hash, 0, 2),
        Some(ForkPosition::BeforeTransaction { block, transaction_index: 1 })
    );

    let before_second = ForkPosition::BeforeTransaction { block, transaction_index: 1 };
    assert_eq!(
        before_second.after_transaction(block, parent_block.hash, 1, 3),
        Some(ForkPosition::BeforeTransaction { block, transaction_index: 2 })
    );
    assert_eq!(
        before_second.after_transaction(block, parent_block.hash, 1, 2),
        Some(ForkPosition::AfterBlock { block })
    );

    assert_eq!(parent.after_transaction(block, B256::ZERO, 0, 1), None);
    assert_eq!(parent.after_transaction(block, parent_block.hash, 1, 2), None);
    assert_eq!(before_second.after_transaction(block, parent_block.hash, 0, 3), None);
    assert_eq!(before_second.after_transaction(block, parent_block.hash, 2, 3), None);
    assert_eq!(before_second.after_transaction(block, parent_block.hash, 1, 1), None);
    assert_eq!(parent.after_transaction(block, parent_block.hash, 0, 0), None);
}

#[test]
fn fork_block_env_updates_slot_number() {
    let mut evm_env = EvmEnv::new(revm::context::CfgEnv::<SpecId>::default(), BlockEnv::default());
    for slot_number in [Some(42), Some(u64::MAX), None, Some(0)] {
        let header = AnyHeader { slot_number, ..Default::default() };
        let block = AnyRpcBlock::new(
            Block::new(
                AnyRpcHeader::from_sealed(header.seal(B256::ZERO)),
                BlockTransactions::Full(Vec::new()),
            )
            .into(),
        );
        update_env_block::<AnyNetwork, _, _>(
            &mut evm_env,
            &block,
            NamedChain::Mainnet as u64,
            NetworkConfigs::default(),
        );
        assert_eq!(evm_env.block_env.slot_num, slot_number.unwrap_or_default());
    }
}

#[test]
fn fork_replay_block_env_preserves_arbitrum_l1_number() {
    let header = AnyHeader { number: 75_219_831, ..Default::default() };
    let mut block = AnyRpcBlock::new(
        Block::new(
            AnyRpcHeader::from_sealed(header.seal(B256::ZERO)),
            BlockTransactions::Full(Vec::new()),
        )
        .into(),
    );
    block.other.insert("l1BlockNumber".to_string(), serde_json::json!("0x10276d3"));
    let mut evm_env = EvmEnv::new(revm::context::CfgEnv::<SpecId>::default(), BlockEnv::default());

    update_env_block::<AnyNetwork, _, _>(
        &mut evm_env,
        &block,
        NamedChain::Arbitrum as u64,
        NetworkConfigs::default(),
    );

    assert_eq!(evm_env.block_env.number, U256::from(16_938_707));
}

#[tokio::test(flavor = "multi_thread")]
async fn temporary_backend_preserves_fork_position() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let block_number = provider.get_block_number().await.unwrap();

    let mut evm_opts = Config::figment().extract::<EvmOpts>().unwrap();
    evm_opts.fork_url = Some(handle.http_endpoint());
    evm_opts.fork_block_number = Some(block_number);
    let fork = evm_opts.get_fork(&Config::default(), 31_337, Some(block_number)).unwrap();
    let mut backend = Backend::<EthEvmNetwork>::spawn(Some(fork)).unwrap();
    let id = backend.active_fork_ids.unwrap().0;
    let fork_id = backend.inner.ensure_fork_id(id).unwrap().clone();

    for position in [
        ForkPosition::BeforeTransaction {
            block: BlockNumHash::new(block_number + 1, B256::with_last_byte(1)),
            transaction_index: 2,
        },
        ForkPosition::AfterBlock {
            block: BlockNumHash::new(block_number + 2, B256::with_last_byte(2)),
        },
    ] {
        backend.inner.get_fork_by_id_mut(id).unwrap().position = position;
        let fork = backend.active_fork().unwrap().clone();
        let journaled_state = fork.journaled_state.clone();
        let mut temporary = Backend::<EthEvmNetwork>::new_with_fork(
            &fork_id,
            fork,
            journaled_state,
            NetworkConfigs::default(),
        )
        .unwrap();

        assert_eq!(temporary.active_fork().unwrap().position, position);
        let expected = match position {
            ForkPosition::AfterBlock { block } | ForkPosition::BeforeTransaction { block, .. } => {
                block.number
            }
        };
        assert_eq!(temporary.active_fork_block_number(), Some(expected));
        temporary.fork_block_number_override = Some(expected + 1);
        assert_eq!(temporary.active_fork_block_number(), Some(expected + 1));
    }
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "monad")]
async fn fork_factory_boundary_preserves_explicit_execution_overrides() {
    async fn pinned_opts(endpoint: String, networks: Option<NetworkConfigs>) -> EvmOpts {
        let mut opts = EvmOpts { fork_url: Some(endpoint), ..Default::default() };
        if let Some(networks) = networks {
            opts.networks = networks;
        }
        opts.infer_network_from_fork().await.unwrap();
        let identity = opts.fork_endpoint.clone().unwrap();
        let network_is_inferred = opts.fork_network_is_inferred;
        opts.expect_fork_endpoint(identity, network_is_inferred);
        opts.pin_fork_block().await.unwrap();
        opts
    }

    fn target_fork(opts: EvmOpts, url: String) -> crate::fork::CreateFork {
        crate::fork::CreateFork { url, enable_caching: false, evm_opts: opts, resolved: None }
    }

    let (ethereum_base_api, ethereum_base) = spawn(NodeConfig::test()).await;
    let (_ethereum_target_api, ethereum_target) = spawn(NodeConfig::test()).await;
    let (monad_base_api, monad_base) = spawn(NodeConfig::test_monad()).await;
    let (_monad_target_api, monad_target) = spawn(NodeConfig::test_monad()).await;
    ethereum_base_api.mine_one().await.unwrap();
    monad_base_api.mine_one().await.unwrap();

    let inferred_ethereum = pinned_opts(ethereum_base.http_endpoint(), None).await;
    assert!(inferred_ethereum.fork_network_is_inferred);
    let error = Backend::<EthEvmNetwork>::spawn(Some(target_fork(
        inferred_ethereum,
        monad_target.http_endpoint(),
    )))
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("cannot create a `monad` fork with an EVM instantiated for `ethereum`"),
        "{error}"
    );

    let inferred_monad = pinned_opts(monad_base.http_endpoint(), None).await;
    assert!(inferred_monad.fork_network_is_inferred);
    let error = Backend::<crate::evm::MonadEvmNetwork>::spawn(Some(target_fork(
        inferred_monad,
        ethereum_target.http_endpoint(),
    )))
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("cannot create a `ethereum` fork with an EVM instantiated for `monad`"),
        "{error}"
    );

    let explicit_ethereum =
        pinned_opts(ethereum_base.http_endpoint(), Some(NetworkConfigs::with_ethereum())).await;
    assert!(!explicit_ethereum.fork_network_is_inferred);
    let _backend = Backend::<EthEvmNetwork>::spawn(Some(target_fork(
        explicit_ethereum,
        monad_target.http_endpoint(),
    )))
    .unwrap();

    let explicit_monad =
        pinned_opts(monad_base.http_endpoint(), Some(NetworkConfigs::with_monad())).await;
    assert!(!explicit_monad.fork_network_is_inferred);
    let _backend = Backend::<crate::evm::MonadEvmNetwork>::spawn(Some(target_fork(
        explicit_monad,
        ethereum_target.http_endpoint(),
    )))
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn can_read_write_cache() {
    let endpoint = &*foundry_test_utils::rpc::next_http_rpc_endpoint();
    let provider = get_http_provider(endpoint);

    let block_num = provider.get_block_number().await.unwrap();

    let mut evm_opts = Config::figment().extract::<EvmOpts>().unwrap();
    evm_opts.fork_url = Some(endpoint.to_string());
    evm_opts.fork_block_number = Some(block_num);

    let (evm_env, _, resolved) = evm_opts.env_resolved::<SpecId, BlockEnv, TxEnv>().await.unwrap();

    let fork = evm_opts
        .get_fork_resolved(&Config::default(), evm_env.cfg_env.chain_id, resolved.as_ref())
        .unwrap();

    let resolved = resolved.unwrap();
    let fork_hash = resolved.hash();
    let source_id = resolved.source_id();
    let backend = Backend::<EthEvmNetwork>::spawn(Some(fork)).unwrap();

    // some rng contract from etherscan
    let address = address!("0x63091244180ae240c87d1f528f5f269134cb07b3");

    let num_slots = 5;
    let _account = backend.basic_ref(address);
    for idx in 0..num_slots {
        let _ = backend.storage_ref(address, U256::from(idx));
    }
    drop(backend);

    let meta = BlockchainDbMeta::new(evm_env.block_env, endpoint.to_string())
        .with_fork_identity(fork_hash, source_id);

    let db = BlockchainDb::new(
        meta,
        Some(Config::foundry_block_cache_dir(NamedChain::Mainnet, block_num).unwrap()),
    );
    assert!(db.accounts().read().contains_key(&address));
    assert!(db.storage().read().contains_key(&address));
    assert_eq!(db.storage().read().get(&address).unwrap().len(), num_slots as usize);
}
