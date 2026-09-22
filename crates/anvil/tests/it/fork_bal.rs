//! Fork cache warming from a second Anvil instance's block access lists.

use alloy_genesis::{Genesis, GenesisAccount};
use alloy_network::TransactionBuilder;
use alloy_primitives::{Address, B256, U256, address, bytes};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, TransactionRequest, anvil::Forking};
use alloy_serde::WithOtherFields;
use anvil::{
    EthereumHardfork, NodeConfig, NodeHandle,
    eth::{
        EthApi,
        backend::db::{SerializableAccountRecord, SerializableState},
    },
    spawn,
};
use foundry_primitives::FoundryNetwork;

mod accounts;

const CONTRACT: Address = address!("000000000000000000000000000000000000ba10");

struct BalOrigin {
    api: EthApi<FoundryNetwork>,
    handle: NodeHandle,
    sender: Address,
    block_number: u64,
    block_hash: B256,
}

impl BalOrigin {
    async fn new() -> Self {
        Self::with_hardfork(EthereumHardfork::Amsterdam).await
    }

    async fn with_hardfork(hardfork: EthereumHardfork) -> Self {
        let (api, handle) = spawn(
            NodeConfig::test()
                .with_chain_id(Some(1u64))
                .with_hardfork(Some(hardfork.into()))
                .with_genesis_timestamp(Some(1_800_000_000u64))
                .with_no_mining(true),
        )
        .await;
        let sender = handle.dev_wallets().next().unwrap().address();
        // Read slot one without changing it, then increment slot zero.
        api.anvil_set_code(CONTRACT, bytes!("6001545060005460010160005500")).await.unwrap();
        api.anvil_set_storage_at(CONTRACT, U256::ONE, B256::from(U256::from(9))).await.unwrap();
        Self::increment(&api, sender).await;
        let block = handle
            .http_provider()
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await
            .unwrap()
            .unwrap();
        Self {
            api,
            handle,
            sender,
            block_number: block.header.number,
            block_hash: block.header.hash,
        }
    }

    fn config(&self) -> NodeConfig {
        NodeConfig::test()
            .with_eth_rpc_url(Some(self.handle.http_endpoint()))
            .with_fork_block_number(Some(self.block_number))
            .with_hardfork(Some(EthereumHardfork::Amsterdam.into()))
            .with_no_storage_caching(true)
            .with_genesis_accounts(vec![])
            .with_no_mining(true)
    }

    async fn increment(api: &EthApi<FoundryNetwork>, sender: Address) {
        api.send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .with_from(sender)
                .with_to(CONTRACT)
                .with_gas_limit(200_000),
        ))
        .await
        .unwrap();
        api.mine_one().await.unwrap();
    }
}

async fn cached_storage(
    api: &EthApi<FoundryNetwork>,
    address: Address,
    slot: U256,
) -> Option<U256> {
    let db = api.backend.get_db().read().await;
    db.maybe_inner()
        .unwrap()
        .storage()
        .read()
        .get(&address)
        .and_then(|slots| slots.get(&slot))
        .copied()
}

async fn has_cached_account(api: &EthApi<FoundryNetwork>, address: Address) -> bool {
    let db = api.backend.get_db().read().await;
    db.maybe_inner().unwrap().accounts().read().contains_key(&address)
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_real_anvil_prefills_changed_storage() {
    let origin = BalOrigin::new().await;
    for no_bal in [false, true] {
        let (api, _handle) = spawn(origin.config().with_no_bal(no_bal)).await;
        assert_eq!(
            cached_storage(&api, CONTRACT, U256::ZERO).await,
            (!no_bal).then_some(U256::ONE)
        );
        assert_eq!(cached_storage(&api, CONTRACT, U256::ONE).await, None);
        assert!(!has_cached_account(&api, CONTRACT).await, "incomplete accounts remain lazy");

        assert_eq!(
            api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
            B256::from(U256::ONE)
        );
        assert_eq!(
            api.storage_at(CONTRACT, U256::ONE, None).await.unwrap(),
            B256::from(U256::from(9))
        );
        assert_eq!(cached_storage(&api, CONTRACT, U256::ONE).await, Some(U256::from(9)));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_unavailable_keeps_storage_lazy() {
    let origin = BalOrigin::with_hardfork(EthereumHardfork::Prague).await;
    assert_eq!(origin.api.block_access_list_by_hash(origin.block_hash).await.unwrap(), None);
    let (api, _handle) = spawn(origin.config()).await;
    assert_eq!(cached_storage(&api, CONTRACT, U256::ZERO).await, None);
    assert_eq!(api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(), B256::from(U256::ONE));
    assert_eq!(cached_storage(&api, CONTRACT, U256::ZERO).await, Some(U256::ONE));
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_prefetches_mutated_anvil_storage_at_current_and_historical_blocks() {
    let origin = BalOrigin::new().await;
    origin
        .api
        .anvil_set_storage_at(CONTRACT, U256::ZERO, B256::from(U256::from(99)))
        .await
        .unwrap();
    for historical in [false, true] {
        if historical {
            origin.api.mine_one().await.unwrap();
        }
        let (api, _handle) = spawn(origin.config()).await;
        assert_eq!(cached_storage(&api, CONTRACT, U256::ZERO).await, Some(U256::from(99)));
        assert_eq!(
            api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
            B256::from(U256::from(99)),
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_matches_lazy_state_across_commits_snapshots_and_reset() {
    let origin = BalOrigin::new().await;
    let mut final_balances = Vec::new();
    for no_bal in [false, true] {
        let (api, _handle) = spawn(origin.config().with_no_bal(no_bal)).await;
        let snapshot = api.evm_snapshot().await.unwrap();
        for value in [2, 3] {
            BalOrigin::increment(&api, origin.sender).await;
            assert_eq!(
                api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
                B256::from(U256::from(value)),
            );
        }
        final_balances.push(api.balance(origin.sender, None).await.unwrap());
        assert!(api.evm_revert(snapshot).await.unwrap());
        let snapshot = api.evm_snapshot().await.unwrap();
        api.anvil_set_storage_at(CONTRACT, U256::ZERO, B256::from(U256::from(77))).await.unwrap();
        assert!(
            api.anvil_reset(Some(Forking {
                json_rpc_url: None,
                block_number: Some(origin.block_number + 100),
            }))
            .await
            .is_err()
        );
        assert_eq!(
            api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
            B256::from(U256::from(77)),
        );
        assert!(api.evm_revert(snapshot).await.unwrap());
        assert_eq!(
            api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
            B256::from(U256::ONE)
        );

        let snapshot = api.evm_snapshot().await.unwrap();
        api.anvil_set_storage_at(CONTRACT, U256::ZERO, B256::from(U256::from(99))).await.unwrap();
        api.anvil_reset(Some(Forking {
            json_rpc_url: None,
            block_number: Some(origin.block_number),
        }))
        .await
        .unwrap();
        assert!(!api.evm_revert(snapshot).await.unwrap());
        assert_eq!(
            api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
            B256::from(U256::ONE)
        );
    }
    assert_eq!(final_balances[0], final_balances[1]);
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_preserves_genesis_funding_and_loaded_state_overrides() {
    let origin = BalOrigin::new().await;
    let genesis = Genesis {
        alloc: [(
            CONTRACT,
            GenesisAccount {
                balance: U256::from(10),
                storage: Some([(B256::ZERO, B256::from(U256::from(20)))].into()),
                ..Default::default()
            },
        )]
        .into(),
        ..Default::default()
    };
    let config = origin
        .config()
        .with_genesis(Some(genesis))
        .with_funded_accounts([(CONTRACT, U256::from(30))].into_iter().collect());
    let (api, handle) = spawn(config.clone()).await;
    assert_eq!(cached_storage(&api, CONTRACT, U256::ZERO).await, Some(U256::ONE));
    assert_eq!(
        api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
        B256::from(U256::from(20))
    );
    assert_eq!(api.balance(CONTRACT, None).await.unwrap(), U256::from(30));
    drop(handle);
    drop(api);

    let state = SerializableState {
        accounts: [(
            CONTRACT,
            SerializableAccountRecord {
                nonce: 3,
                balance: U256::from(40),
                code: bytes!("00"),
                storage: [(B256::ZERO, B256::from(U256::from(50)))].into(),
            },
        )]
        .into(),
        ..Default::default()
    };
    let (api, _handle) = spawn(config.with_init_state(Some(state))).await;
    assert_eq!(
        api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
        B256::from(U256::from(50))
    );
    assert_eq!(api.balance(CONTRACT, None).await.unwrap(), U256::from(40));
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_reset_refreshes_the_remote_seed() {
    let origin = BalOrigin::new().await;
    let (api, _handle) = spawn(origin.config()).await;
    assert_eq!(cached_storage(&api, CONTRACT, U256::ZERO).await, Some(U256::ONE));

    BalOrigin::increment(&origin.api, origin.sender).await;
    api.anvil_reset(Some(Forking {
        json_rpc_url: None,
        block_number: Some(origin.block_number + 1),
    }))
    .await
    .unwrap();
    assert_eq!(cached_storage(&api, CONTRACT, U256::ZERO).await, Some(U256::from(2)));
    assert_eq!(
        api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
        B256::from(U256::from(2))
    );
    api.anvil_reset(None).await.unwrap();
    assert_eq!(api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(), B256::ZERO);
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_transaction_hash_uses_parent_seed_and_replays_target_prefix() {
    let origin = BalOrigin::new().await;
    let nonce = origin.handle.http_provider().get_transaction_count(origin.sender).await.unwrap();
    let mut transactions = Vec::new();
    for nonce in [nonce, nonce + 1] {
        transactions.push(
            origin
                .api
                .send_transaction(WithOtherFields::new(
                    TransactionRequest::default()
                        .with_from(origin.sender)
                        .with_to(CONTRACT)
                        .with_gas_limit(200_000)
                        .with_nonce(nonce),
                ))
                .await
                .unwrap(),
        );
    }
    origin.api.mine_one().await.unwrap();
    assert_eq!(
        origin.api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
        B256::from(U256::from(3)),
    );

    let (api, _handle) =
        spawn(origin.config().with_fork_transaction_hash(Some(transactions[0]))).await;
    assert_eq!(cached_storage(&api, CONTRACT, U256::ZERO).await, Some(U256::ONE));
    assert_eq!(
        api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
        B256::from(U256::from(2))
    );
    assert!(api.backend.mined_transaction_by_hash(transactions[0]).is_some());
    assert!(api.backend.mined_transaction_by_hash(transactions[1]).is_none());
}
