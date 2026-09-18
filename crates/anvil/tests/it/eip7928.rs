//! EIP-7928 block access list tests.
//!
//! Anvil builds block access lists for locally mined Amsterdam blocks and forwards requests for
//! blocks that predate a fork to the upstream node.

use alloy_eips::eip7928::{BlockAccessIndex, BlockAccessList, compute_block_access_list_hash};
use alloy_network::{Network, TransactionBuilder};
use alloy_primitives::{B256, Bytes, U256};
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
async fn block_access_list_is_null_for_non_ethereum_execution() {
    let (api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(EthereumHardfork::Amsterdam.into())))
            .await;
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

    let (_api, handle) = spawn(NodeConfig::test().with_eth_rpc_url(Some(proxy))).await;
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
