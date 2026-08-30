//! EIP-7928 block access list tests.

use alloy_primitives::B256;
use alloy_rpc_types::BlockNumberOrTag;
use anvil::{NodeConfig, spawn};
use foundry_test_utils::rpc::spawn_rpc_proxy_canned_method;
use serde_json::json;
use std::sync::atomic::Ordering;

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
