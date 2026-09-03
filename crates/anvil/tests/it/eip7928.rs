//! EIP-7928 block access list tests.
//!
//! Anvil does not build block access lists itself; the endpoints exist for fork compatibility and
//! forward to the forked node for blocks that predate the fork. Everything else answers `null`.

use alloy_network::Network;
use alloy_primitives::{B256, Bytes};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, BlockNumberOrTag};
use anvil::{NodeConfig, spawn};
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
async fn block_access_list_is_null_without_a_fork() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    api.mine_one().await.unwrap();
    let block = provider.get_block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();

    assert_all_null(&provider, block.header.hash, block.header.number).await;
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
