use crate::{
    backend::{Backend, DatabaseExt, ForkPosition, ReplayInputs, update_env_block},
    evm::EthEvmNetwork,
    fork::CreateFork,
    opts::EvmOpts,
};
use alloy_consensus::BlockHeader;
use alloy_eips::{
    BlockId, BlockNumHash,
    eip2935::{HISTORY_STORAGE_ADDRESS, HISTORY_STORAGE_CODE},
    eip4788::BEACON_ROOTS_ADDRESS,
    eip7928::{
        AccountChanges, BalanceChange, BlockAccessIndex, BlockAccessList, NonceChange, SlotChanges,
        StorageChange,
    },
};
use alloy_network::{AnyNetwork, AnyRpcBlock, BlockResponse, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, U256, hex};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, TransactionRequest};
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, NodeHandle, spawn};
use foundry_evm_networks::NetworkConfigs;
use foundry_test_utils::rpc::spawn_rpc_proxy_canned_method;
use revm::{
    DatabaseRef,
    context::{JournalInner, TxEnv},
    primitives::hardfork::SpecId,
    state::EvmStorageSlot,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::atomic::Ordering};

const BAL_METHOD: &str = "eth_getBlockAccessListByBlockHash";

struct Fixture {
    handle: NodeHandle,
    block: AnyRpcBlock,
    hashes: [B256; 3],
    recipient: Address,
    beacon_value: U256,
    bal: BlockAccessList,
}

impl Fixture {
    async fn new() -> Self {
        let (api, handle) = spawn(NodeConfig::test()).await;
        let provider = handle.http_provider();
        let sender = provider.get_accounts().await.unwrap()[0];
        let recipient = Address::with_last_byte(0x92);
        api.anvil_set_code(BEACON_ROOTS_ADDRESS, hex!("60005460010160005500").into())
            .await
            .unwrap();
        api.anvil_set_nonce(BEACON_ROOTS_ADDRESS, U256::from(1)).await.unwrap();
        api.anvil_set_code(HISTORY_STORAGE_ADDRESS, HISTORY_STORAGE_CODE.clone()).await.unwrap();
        api.anvil_set_nonce(HISTORY_STORAGE_ADDRESS, U256::from(1)).await.unwrap();
        api.mine_one().await.unwrap();
        let parent = provider.get_block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
        let parent_hash = parent.header.hash;
        let beacon_value = provider.get_storage_at(BEACON_ROOTS_ADDRESS, U256::ZERO).await.unwrap()
            + U256::from(1);
        let nonce = provider.get_transaction_count(sender).await.unwrap();
        let gas_price = provider.get_gas_price().await.unwrap();
        api.anvil_set_auto_mine(false).await.unwrap();
        let mut hashes = [B256::ZERO; 3];
        for (index, hash) in hashes.iter_mut().enumerate() {
            *hash = api
                .send_transaction(WithOtherFields::new(
                    TransactionRequest::default()
                        .with_from(sender)
                        .with_to(recipient)
                        .with_value(U256::from(7))
                        .with_nonce(nonce + index as u64)
                        .with_gas_limit(21_000)
                        .with_gas_price(gas_price),
                ))
                .await
                .unwrap();
        }
        api.mine_one().await.unwrap();
        let block =
            provider.get_block_by_number(BlockNumberOrTag::Latest).full().await.unwrap().unwrap();
        let block =
            serde_json::from_value::<AnyRpcBlock>(serde_json::to_value(block).unwrap()).unwrap();
        let beneficiary = block.header().beneficiary();
        let mut accounts = BTreeMap::from_iter(
            [sender, beneficiary, recipient].map(|address| (address, AccountChanges::new(address))),
        );
        let mut sender_balance =
            provider.get_balance(sender).block_id(BlockId::hash(parent_hash)).await.unwrap();
        let mut beneficiary_balance =
            provider.get_balance(beneficiary).block_id(BlockId::hash(parent_hash)).await.unwrap();
        for (index, hash) in hashes.iter().enumerate() {
            let receipt = provider.get_transaction_receipt(*hash).await.unwrap().unwrap();
            let access_index = BlockAccessIndex::new(index as u64 + 1);
            sender_balance -= U256::from(7)
                + U256::from(receipt.gas_used()) * U256::from(receipt.effective_gas_price());
            beneficiary_balance += U256::from(receipt.gas_used())
                * U256::from(
                    receipt.effective_gas_price()
                        - block.header().base_fee_per_gas().unwrap() as u128,
                );
            accounts
                .get_mut(&sender)
                .unwrap()
                .balance_changes
                .push(BalanceChange::new(access_index, sender_balance));
            accounts
                .get_mut(&sender)
                .unwrap()
                .nonce_changes
                .push(NonceChange::new(access_index, nonce + index as u64 + 1));
            accounts
                .get_mut(&beneficiary)
                .unwrap()
                .balance_changes
                .push(BalanceChange::new(access_index, beneficiary_balance));
            accounts
                .get_mut(&recipient)
                .unwrap()
                .balance_changes
                .push(BalanceChange::new(access_index, U256::from(7 * (index + 1))));
        }
        for (address, slot, value) in [
            (BEACON_ROOTS_ADDRESS, U256::ZERO, beacon_value),
            (
                HISTORY_STORAGE_ADDRESS,
                U256::from(parent.header.number % 8191),
                U256::from_be_bytes(parent_hash.0),
            ),
        ] {
            accounts.insert(
                address,
                AccountChanges {
                    storage_changes: vec![SlotChanges::new(
                        slot,
                        vec![StorageChange::new(BlockAccessIndex::new(0), value)],
                    )],
                    ..AccountChanges::new(address)
                },
            );
        }
        Self {
            handle,
            block,
            hashes,
            recipient,
            beacon_value,
            bal: accounts.into_values().collect(),
        }
    }

    fn fork(&self, endpoint: String, parent: bool) -> CreateFork {
        CreateFork {
            url: endpoint.clone(),
            enable_caching: false,
            evm_opts: EvmOpts {
                fork_url: Some(endpoint),
                fork_block_number: Some(self.block.header().number() - u64::from(parent)),
                ..Default::default()
            },
            resolved: None,
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn transaction_fork_bal_applies_full_state_and_refreshes_journals() {
    let fixture = Fixture::new().await;
    let (endpoint, calls) = spawn_rpc_proxy_canned_method(
        fixture.handle.http_endpoint(),
        BAL_METHOD,
        serde_json::to_value(&fixture.bal).unwrap(),
    )
    .await;
    let transactions = fixture.block.transactions().as_transactions().unwrap();
    for (index, target) in transactions.iter().enumerate() {
        let mut backend =
            Backend::<EthEvmNetwork>::spawn(Some(fixture.fork(endpoint.clone(), true))).unwrap();
        let id = backend.active_fork_id().unwrap();
        let fork_id = backend.inner.ensure_fork_id(id).unwrap().clone();
        let mut evm_env = backend.forks.get_evm_env(fork_id.clone()).unwrap().unwrap();
        update_env_block::<AnyNetwork, _, _>(
            &mut evm_env,
            &fixture.block,
            31_337,
            backend.networks,
        );
        let replay = ReplayInputs {
            fork_id,
            forks: backend.forks.clone(),
            evm_env,
            networks: backend.networks,
        };
        let mut journal = JournalInner::new();
        journal.load_account(&mut backend, BEACON_ROOTS_ADDRESS).unwrap();
        journal.state.get_mut(&BEACON_ROOTS_ADDRESS).unwrap().storage.insert(
            U256::ZERO,
            EvmStorageSlot::new(fixture.beacon_value - U256::from(1), Default::default()),
        );
        let persistent = backend.inner.persistent_accounts.clone();
        let fork = backend.inner.get_fork_by_id_mut(id).unwrap();
        fork.journaled_state = journal.clone();
        assert!(
            Backend::<EthEvmNetwork>::try_apply_bal_prestate(
                fork,
                &replay,
                &fixture.block,
                target,
                &mut journal,
                &persistent,
            )
            .unwrap()
        );
        assert_eq!(
            fork.db.basic_ref(fixture.recipient).unwrap().unwrap_or_default().balance,
            U256::from(7 * index)
        );
        assert_eq!(
            fork.db.storage_ref(BEACON_ROOTS_ADDRESS, U256::ZERO).unwrap(),
            fixture.beacon_value
        );
        assert_eq!(
            journal.state[&BEACON_ROOTS_ADDRESS].storage[&U256::ZERO].present_value(),
            fixture.beacon_value
        );
        assert_eq!(
            fork.journaled_state.state[&BEACON_ROOTS_ADDRESS].storage[&U256::ZERO].present_value(),
            fixture.beacon_value
        );
        assert_eq!(
            fork.db.db.storage_ref(BEACON_ROOTS_ADDRESS, U256::ZERO).unwrap(),
            fixture.beacon_value - U256::from(1)
        );
    }
    assert_eq!(calls.load(Ordering::Relaxed), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn transaction_fork_bal_and_fallback_preserve_prefix_boundaries() {
    let fixture = Fixture::new().await;
    let mut incomplete = fixture.bal.clone();
    for account in &mut incomplete {
        account.nonce_changes.clear();
    }
    for response in [
        serde_json::to_value(&fixture.bal).unwrap(),
        Value::Null,
        json!([]),
        serde_json::to_value(incomplete).unwrap(),
    ] {
        let (endpoint, calls) =
            spawn_rpc_proxy_canned_method(fixture.handle.http_endpoint(), BAL_METHOD, response)
                .await;
        for (index, hash) in fixture.hashes.iter().enumerate() {
            let mut backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
            let id = backend
                .create_fork_at_transaction(fixture.fork(endpoint.clone(), false), *hash)
                .unwrap();
            let fork = backend.inner.get_fork_by_id(id).unwrap();
            assert_eq!(
                fork.position,
                ForkPosition::BeforeTransaction {
                    block: BlockNumHash::new(
                        fixture.block.header().number(),
                        fixture.block.header().hash
                    ),
                    transaction_index: index,
                }
            );
            assert_eq!(
                fork.db.basic_ref(fixture.recipient).unwrap().unwrap_or_default().balance,
                U256::from(7 * index)
            );
            assert_eq!(
                fork.db.storage_ref(BEACON_ROOTS_ADDRESS, U256::ZERO).unwrap(),
                fixture.beacon_value
            );
            let parent_number = fixture.block.header().number() - 1;
            assert_eq!(
                fork.db
                    .storage_ref(HISTORY_STORAGE_ADDRESS, U256::from(parent_number % 8191))
                    .unwrap(),
                U256::from_be_bytes(fixture.block.header().parent_hash().0)
            );
        }
        assert_eq!(calls.load(Ordering::Relaxed), 3);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn transaction_fork_bal_preserves_persistent_prefix_overrides() {
    let fixture = Fixture::new().await;
    let (endpoint, calls) = spawn_rpc_proxy_canned_method(
        fixture.handle.http_endpoint(),
        BAL_METHOD,
        serde_json::to_value(&fixture.bal).unwrap(),
    )
    .await;
    let mut backend = Backend::<EthEvmNetwork>::spawn(Some(fixture.fork(endpoint, true))).unwrap();
    let id = backend.active_fork_id().unwrap();
    let fork_id = backend.inner.ensure_fork_id(id).unwrap().clone();
    let mut evm_env = backend.forks.get_evm_env(fork_id).unwrap().unwrap();
    let mut info = backend.basic_ref(fixture.recipient).unwrap().unwrap_or_default();
    info.balance = U256::from(200);
    backend.insert_account_info(fixture.recipient, info);
    backend.add_persistent_account(fixture.recipient);
    let mut journal = JournalInner::new();
    backend
        .roll_fork_to_transaction(
            Some(id),
            fixture.hashes[1],
            &mut evm_env,
            &TxEnv::default(),
            &mut journal,
        )
        .unwrap();
    assert_eq!(backend.basic_ref(fixture.recipient).unwrap().unwrap().balance, U256::from(207));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn transaction_fork_bal_skips_execution_overrides_before_probing() {
    let fixture = Fixture::new().await;
    let (endpoint, calls) = spawn_rpc_proxy_canned_method(
        fixture.handle.http_endpoint(),
        BAL_METHOD,
        serde_json::to_value(&fixture.bal).unwrap(),
    )
    .await;
    for override_chain_id in [false, true] {
        let mut backend =
            Backend::<EthEvmNetwork>::spawn(Some(fixture.fork(endpoint.clone(), true))).unwrap();
        let id = backend.active_fork_id().unwrap();
        let fork_id = backend.inner.ensure_fork_id(id).unwrap().clone();
        let mut evm_env = backend.forks.get_evm_env(fork_id.clone()).unwrap().unwrap();
        update_env_block::<AnyNetwork, _, _>(
            &mut evm_env,
            &fixture.block,
            31_337,
            backend.networks,
        );
        if override_chain_id {
            evm_env.cfg_env.chain_id += 1;
        } else {
            evm_env.cfg_env.set_spec_and_mainnet_gas_params(SpecId::CANCUN);
        }
        let replay = ReplayInputs {
            fork_id,
            forks: backend.forks.clone(),
            evm_env,
            networks: backend.networks,
        };
        let persistent = backend.inner.persistent_accounts.clone();
        let fork = backend.inner.get_fork_by_id_mut(id).unwrap();
        assert!(
            !Backend::<EthEvmNetwork>::try_apply_bal_prestate(
                fork,
                &replay,
                &fixture.block,
                &fixture.block.transactions().as_transactions().unwrap()[1],
                &mut JournalInner::new(),
                &persistent,
            )
            .unwrap()
        );
        assert_eq!(
            fork.db.basic_ref(fixture.recipient).unwrap().unwrap_or_default().balance,
            U256::ZERO
        );
    }
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn transaction_fork_bal_failed_proof_keeps_parent_state() {
    let fixture = Fixture::new().await;
    let (proof_endpoint, proof_calls) =
        spawn_rpc_proxy_canned_method(fixture.handle.http_endpoint(), "eth_getProof", Value::Null)
            .await;
    let (endpoint, _) = spawn_rpc_proxy_canned_method(
        proof_endpoint,
        BAL_METHOD,
        serde_json::to_value(&fixture.bal).unwrap(),
    )
    .await;
    let mut backend =
        Backend::<EthEvmNetwork>::spawn(Some(fixture.fork(endpoint.clone(), true))).unwrap();
    let id = backend.active_fork_id().unwrap();
    let fork_id = backend.inner.ensure_fork_id(id).unwrap().clone();
    let mut evm_env = backend.forks.get_evm_env(fork_id.clone()).unwrap().unwrap();
    update_env_block::<AnyNetwork, _, _>(&mut evm_env, &fixture.block, 31_337, backend.networks);
    let replay =
        ReplayInputs { fork_id, forks: backend.forks.clone(), evm_env, networks: backend.networks };
    let mut journal = JournalInner::new();
    journal.load_account(&mut backend, BEACON_ROOTS_ADDRESS).unwrap();
    journal.state.get_mut(&BEACON_ROOTS_ADDRESS).unwrap().storage.insert(
        U256::ZERO,
        EvmStorageSlot::new(fixture.beacon_value - U256::from(1), Default::default()),
    );
    let persistent = backend.inner.persistent_accounts.clone();
    let fork = backend.inner.get_fork_by_id_mut(id).unwrap();
    fork.journaled_state = journal.clone();
    assert!(
        Backend::<EthEvmNetwork>::try_apply_bal_prestate(
            fork,
            &replay,
            &fixture.block,
            &fixture.block.transactions().as_transactions().unwrap()[1],
            &mut journal,
            &persistent,
        )
        .is_err()
    );
    assert_eq!(
        fork.db.basic_ref(fixture.recipient).unwrap().unwrap_or_default().balance,
        U256::ZERO
    );
    assert_eq!(
        fork.db.storage_ref(BEACON_ROOTS_ADDRESS, U256::ZERO).unwrap(),
        fixture.beacon_value - U256::from(1)
    );
    assert_eq!(
        journal.state[&BEACON_ROOTS_ADDRESS].storage[&U256::ZERO].present_value(),
        fixture.beacon_value - U256::from(1)
    );
    assert_eq!(
        fork.journaled_state.state[&BEACON_ROOTS_ADDRESS].storage[&U256::ZERO].present_value(),
        fixture.beacon_value - U256::from(1)
    );
    assert!(proof_calls.load(Ordering::Relaxed) > 0);

    let id = backend
        .create_fork_at_transaction(fixture.fork(endpoint, false), fixture.hashes[1])
        .unwrap();
    let fork = backend.inner.get_fork_by_id(id).unwrap();
    assert_eq!(fork.db.basic_ref(fixture.recipient).unwrap().unwrap().balance, U256::from(7));
    assert_eq!(
        fork.db.storage_ref(BEACON_ROOTS_ADDRESS, U256::ZERO).unwrap(),
        fixture.beacon_value
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn transaction_fork_system_calls_use_source_chain_and_execution_rules() {
    let fixture = Fixture::new().await;
    let backend =
        Backend::<EthEvmNetwork>::spawn(Some(fixture.fork(fixture.handle.http_endpoint(), true)))
            .unwrap();
    let fork_id = backend.inner.ensure_fork_id(backend.active_fork_id().unwrap()).unwrap().clone();
    let evm_env = backend.forks.get_evm_env(fork_id.clone()).unwrap().unwrap();
    let mut replay =
        ReplayInputs { fork_id, forks: backend.forks.clone(), evm_env, networks: backend.networks };
    replay.evm_env.cfg_env.set_spec_and_mainnet_gas_params(SpecId::PRAGUE);
    // A chain-ID override does not change the source chain's pre-block operations.
    replay.evm_env.cfg_env.chain_id = 42_161;
    assert_eq!(
        Backend::<EthEvmNetwork>::pre_block_system_calls(&replay, &fixture.block, 31_337)
            .into_iter()
            .map(|(address, _)| address)
            .collect::<Vec<_>>(),
        [HISTORY_STORAGE_ADDRESS, BEACON_ROOTS_ADDRESS]
    );
    replay.evm_env.cfg_env.chain_id = 1;
    assert!(
        Backend::<EthEvmNetwork>::pre_block_system_calls(&replay, &fixture.block, 42_161)
            .is_empty()
    );
    for networks in [NetworkConfigs::with_celo(), NetworkConfigs::with_tempo()] {
        replay.networks = networks;
        assert!(
            Backend::<EthEvmNetwork>::pre_block_system_calls(&replay, &fixture.block, 31_337)
                .is_empty()
        );
    }
    replay.networks = NetworkConfigs::default();
    replay.evm_env.cfg_env.set_spec_and_mainnet_gas_params(SpecId::SHANGHAI);
    assert!(
        Backend::<EthEvmNetwork>::pre_block_system_calls(&replay, &fixture.block, 31_337)
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn transaction_fork_replay_applies_pre_block_system_state() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let sender = provider.get_accounts().await.unwrap()[0];
    api.anvil_set_code(BEACON_ROOTS_ADDRESS, Bytes::from_static(&hex!("60005460010160005500")))
        .await
        .unwrap();
    api.anvil_set_nonce(BEACON_ROOTS_ADDRESS, U256::from(1)).await.unwrap();
    api.mine_one().await.unwrap();
    let before = provider.get_storage_at(BEACON_ROOTS_ADDRESS, U256::ZERO).await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();
    api.anvil_set_auto_mine(false).await.unwrap();
    let target = api
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .with_from(sender)
                .with_to(Address::with_last_byte(0x91))
                .with_value(U256::from(1))
                .with_gas_limit(21_000)
                .with_gas_price(gas_price),
        ))
        .await
        .unwrap();
    api.mine_one().await.unwrap();
    provider.get_transaction_receipt(target).await.unwrap().unwrap();
    assert_eq!(
        provider.get_storage_at(BEACON_ROOTS_ADDRESS, U256::ZERO).await.unwrap(),
        before + U256::from(1),
    );
    let endpoint = handle.http_endpoint();
    let fork = CreateFork {
        url: endpoint.clone(),
        enable_caching: false,
        evm_opts: EvmOpts { fork_url: Some(endpoint), ..Default::default() },
        resolved: None,
    };
    let mut backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
    let id = backend.create_fork_at_transaction(fork, target).unwrap();
    let fork = backend.inner.get_fork_by_id(id).unwrap();
    assert_eq!(
        fork.db.storage_ref(BEACON_ROOTS_ADDRESS, U256::ZERO).unwrap(),
        before + U256::from(1),
    );
}
