//! EIP-7928 block access list tests.
//!
//! Anvil builds block access lists for locally mined Amsterdam blocks and forwards requests for
//! blocks that predate a fork to the upstream node.

use crate::abi::{COUNTER_INIT_CODE, COUNTER_RUNTIME_CODE};
use alloy_eips::{
    eip2935::HISTORY_STORAGE_ADDRESS,
    eip4788::BEACON_ROOTS_ADDRESS,
    eip7002::WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
    eip7928::{
        AccountChanges, BlockAccessIndex, BlockAccessList, CodeChange, NonceChange, SlotChanges,
        StorageChange, compute_block_access_list_hash, validate_block_access_list,
    },
};
use alloy_network::{Network, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, BlockNumberOrTag, TransactionRequest};
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, spawn};
use foundry_evm::hardfork::EthereumHardfork;
use foundry_test_utils::rpc::spawn_rpc_proxy_canned_method;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;

/// The four endpoints, paired with the params each takes for a block that exists.
async fn assert_all_null<N: Network>(provider: &impl Provider<N>, hash: B256, number: u64) {
    let by_id: Option<Value> = provider
        .client()
        .request("eth_getBlockAccessList", (BlockId::number(number),))
        .await
        .unwrap();
    assert_eq!(by_id, None);

    let by_hash: Option<Value> =
        provider.client().request("eth_getBlockAccessListByBlockHash", (hash,)).await.unwrap();
    assert_eq!(by_hash, None);

    let by_number: Option<Value> = provider
        .client()
        .request("eth_getBlockAccessListByBlockNumber", (BlockNumberOrTag::Number(number),))
        .await
        .unwrap();
    assert_eq!(by_number, None);

    let raw: Option<Bytes> = provider
        .client()
        .request("eth_getBlockAccessListRaw", (BlockId::number(number),))
        .await
        .unwrap();
    assert_eq!(raw, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn block_access_list_is_null_before_amsterdam() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    api.mine_one().await.unwrap();
    let block = provider.get_block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();

    assert_all_null(&provider, block.header.hash, block.header.number).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn locally_mined_amsterdam_block_has_access_list() {
    let (api, handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Amsterdam.into()))).await;
    let provider = handle.http_provider();

    api.mine_one().await.unwrap();
    let block = provider.get_block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    let number = block.header.number;
    let hash = block.header.hash;

    let by_id: BlockAccessList = provider
        .client()
        .request::<_, Option<BlockAccessList>>("eth_getBlockAccessList", (BlockId::number(number),))
        .await
        .unwrap()
        .unwrap();
    assert!(!by_id.is_empty(), "Amsterdam system calls should produce BAL entries");

    let by_hash = provider.get_block_access_list_by_hash(hash).await.unwrap().unwrap();
    let by_number = provider
        .get_block_access_list_by_number(BlockNumberOrTag::Number(number))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(by_hash, by_id);
    assert_eq!(by_number, by_id);

    let raw = provider.get_block_access_list_raw(BlockId::number(number)).await.unwrap().unwrap();
    let mut expected_raw = Vec::new();
    alloy_rlp::encode_list(&by_id, &mut expected_raw);
    assert_eq!(raw.as_ref(), expected_raw);
    assert_eq!(block.header.block_access_list_hash, Some(compute_block_access_list_hash(&by_id)));
}

#[tokio::test(flavor = "multi_thread")]
async fn locally_mined_transactions_use_ordered_bal_indices() {
    let (api, handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Amsterdam.into()))).await;
    let provider = handle.http_provider();
    api.anvil_set_auto_mine(false).await.unwrap();
    let accounts = provider.get_accounts().await.unwrap();
    let from = accounts[0];
    let to = accounts[1];

    for nonce in 0..2 {
        let tx = TransactionRequest::default()
            .with_from(from)
            .with_to(to)
            .with_value(U256::from(nonce + 1))
            .with_nonce(nonce);
        let _ = provider.send_transaction(WithOtherFields::new(tx)).await.unwrap();
    }
    api.mine_one().await.unwrap();

    let bal =
        provider.get_block_access_list_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    let sender = bal.iter().find(|account| account.address == from).unwrap();
    let indices =
        sender.nonce_changes.iter().map(|change| change.block_access_index).collect::<Vec<_>>();
    assert_eq!(indices, [BlockAccessIndex::new(1), BlockAccessIndex::new(2)]);
}

fn account(bal: &BlockAccessList, address: Address) -> &AccountChanges {
    bal.iter().find(|account| account.address == address).unwrap_or_else(|| {
        panic!("{address} is missing from the block access list");
    })
}

fn storage_indices(account: &AccountChanges) -> Vec<BlockAccessIndex> {
    account
        .storage_changes
        .iter()
        .flat_map(|slot| slot.changes.iter().map(|change| change.block_access_index))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn locally_mined_block_access_list_records_every_phase() {
    let (api, handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Amsterdam.into()))).await;
    let provider = handle.http_provider();
    api.anvil_set_auto_mine(false).await.unwrap();
    let sender = handle.dev_wallets().next().unwrap().address();
    let nonce = provider.get_transaction_count(sender).await.unwrap();
    let contract = sender.create(nonce);

    let deploy = TransactionRequest::default()
        .with_from(sender)
        .with_nonce(nonce)
        .with_deploy_code(COUNTER_INIT_CODE)
        .with_gas_limit(1_000_000);
    let call = TransactionRequest::default()
        .with_from(sender)
        .with_nonce(nonce + 1)
        .with_to(contract)
        .with_gas_limit(1_000_000);
    let _ = provider.send_transaction(WithOtherFields::new(deploy)).await.unwrap();
    let _ = provider.send_transaction(WithOtherFields::new(call)).await.unwrap();
    api.mine_one().await.unwrap();

    let block = provider.get_block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert_eq!(block.transactions.len(), 2);
    assert_eq!(provider.get_storage_at(contract, U256::ZERO).await.unwrap(), U256::from(2));
    let bal = provider.get_block_access_list_by_hash(block.header.hash).await.unwrap().unwrap();
    validate_block_access_list(&bal, 2).unwrap();
    assert_eq!(block.header.block_access_list_hash, Some(compute_block_access_list_hash(&bal)));

    // Transactions occupy indices 1 and 2.
    let index = |index: u64| BlockAccessIndex::new(index);
    let sender_changes = account(&bal, sender);
    assert_eq!(
        sender_changes.nonce_changes,
        [NonceChange::new(index(1), nonce + 1), NonceChange::new(index(2), nonce + 2)]
    );
    assert_eq!(
        sender_changes
            .balance_changes
            .iter()
            .map(|change| change.block_access_index)
            .collect::<Vec<_>>(),
        [index(1), index(2)]
    );
    let beneficiary = account(&bal, block.header.beneficiary);
    assert_eq!(
        beneficiary
            .balance_changes
            .iter()
            .map(|change| change.block_access_index)
            .collect::<Vec<_>>(),
        [index(1), index(2)]
    );
    let contract_changes = account(&bal, contract);
    assert_eq!(contract_changes.nonce_changes, [NonceChange::new(index(1), 1)]);
    assert_eq!(contract_changes.code_changes, [CodeChange::new(index(1), COUNTER_RUNTIME_CODE)]);
    assert_eq!(
        contract_changes.storage_changes,
        [SlotChanges::new(
            U256::ZERO,
            vec![
                StorageChange::new(index(1), U256::from(1)),
                StorageChange::new(index(2), U256::from(2)),
            ],
        )]
    );

    // Pre-block system calls write at index 0, post-block system calls after the last
    // transaction.
    for system_contract in [BEACON_ROOTS_ADDRESS, HISTORY_STORAGE_ADDRESS] {
        let indices = storage_indices(account(&bal, system_contract));
        assert!(!indices.is_empty(), "{system_contract} should record pre-block writes");
        assert!(indices.iter().all(|i| *i == index(0)), "{system_contract}: {indices:?}");
    }
    let indices = storage_indices(account(&bal, WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS));
    assert!(indices.iter().all(|i| *i == index(3)), "{indices:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn reverting_snapshot_removes_local_block_access_list() {
    let (api, handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Amsterdam.into()))).await;
    let snapshot = api.evm_snapshot().await.unwrap();
    api.mine_one().await.unwrap();
    let block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert!(api.block_access_list_by_hash(block.header.hash).await.unwrap().is_some());

    assert!(api.evm_revert(snapshot).await.unwrap());
    assert_eq!(api.block_access_list_by_hash(block.header.hash).await.unwrap(), None);
    assert_eq!(handle.http_provider().get_block_number().await.unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn block_access_list_rejects_out_of_range_blocks() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    // Block resolution happens before the fork check, so an out-of-range number is an error
    // rather than a `null` access list.
    for method in ["eth_getBlockAccessList", "eth_getBlockAccessListRaw"] {
        let err = provider
            .client()
            .request::<_, Option<Value>>(method, (BlockId::number(9999),))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("BlockOutOfRangeError"), "{method}: unexpected {err}");
    }

    // An unknown hash has no range to check and simply reports no access list.
    let by_hash: Option<Value> = provider
        .client()
        .request("eth_getBlockAccessListByBlockHash", (B256::random(),))
        .await
        .unwrap();
    assert_eq!(by_hash, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_block_access_list_forwards_pre_fork_blocks() {
    let (_origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let canned = json!({ "blockAccessList": [] });
    let (proxy, calls) = spawn_rpc_proxy_canned_method(
        origin_handle.http_endpoint(),
        "eth_getBlockAccessList",
        canned.clone(),
    )
    .await;

    let (_api, handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(proxy)).with_no_bal(true)).await;
    let provider = handle.http_provider();

    // The fork block itself predates the fork, so the request reaches the upstream node.
    let fork_block = provider.get_block_number().await.unwrap();
    let forwarded: Option<Value> = provider
        .client()
        .request("eth_getBlockAccessList", (BlockId::number(fork_block),))
        .await
        .unwrap();
    assert_eq!(forwarded, Some(canned));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_block_access_list_skips_locally_mined_blocks() {
    let (_origin_api, origin) = spawn(NodeConfig::test()).await;
    let (fork_url, upstream_calls) = spawn_rpc_proxy_canned_method(
        origin.http_endpoint(),
        "eth_getBlockAccessListByBlockNumber",
        json!({"blockAccessList": []}),
    )
    .await;
    let (api, _handle) = spawn(NodeConfig::test().with_eth_rpc_url(Some(fork_url))).await;

    api.mine_one().await.unwrap();
    let number = api.block_number().unwrap().to::<u64>();
    assert!(number > 0, "expected a locally mined block");

    let access_list =
        api.block_access_list_by_number(BlockNumberOrTag::Number(number)).await.unwrap();

    assert_eq!(access_list, None);
    assert_eq!(upstream_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_block_access_list_by_hash_skips_locally_mined_blocks() {
    let (_origin_api, origin) = spawn(NodeConfig::test()).await;
    let (fork_url, upstream_calls) = spawn_rpc_proxy_canned_method(
        origin.http_endpoint(),
        "eth_getBlockAccessListByBlockHash",
        json!({"blockAccessList": []}),
    )
    .await;
    let (api, _handle) = spawn(NodeConfig::test().with_eth_rpc_url(Some(fork_url))).await;

    api.mine_one().await.unwrap();
    let block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert!(block.header.number > 0, "expected a locally mined block");

    let access_list = api.block_access_list_by_hash(block.header.hash).await.unwrap();

    assert_eq!(access_list, None);
    assert_eq!(upstream_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_block_access_list_by_hash_forwards_unknown_blocks() {
    let (_origin_api, origin) = spawn(NodeConfig::test()).await;
    let (fork_url, upstream_calls) = spawn_rpc_proxy_canned_method(
        origin.http_endpoint(),
        "eth_getBlockAccessListByBlockHash",
        json!({"blockAccessList": []}),
    )
    .await;
    let (api, _handle) = spawn(NodeConfig::test().with_eth_rpc_url(Some(fork_url))).await;

    let access_list = api.block_access_list_by_hash(B256::random()).await.unwrap();

    assert_eq!(access_list, Some(json!({"blockAccessList": []})));
    assert_eq!(upstream_calls.load(Ordering::Relaxed), 1);
}
