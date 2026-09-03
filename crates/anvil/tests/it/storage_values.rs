//! Tests for the `eth_getStorageValues` batch storage endpoint.

use alloy_primitives::{Address, B256, U256, map::HashMap};
use alloy_provider::Provider;
use alloy_rpc_types::BlockId;
use anvil::{NodeConfig, spawn};
use foundry_test_utils::rpc::spawn_rpc_proxy_canned_method;
use std::sync::atomic::Ordering;

fn slot(n: u64) -> B256 {
    B256::from(U256::from(n))
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_values_batches_multiple_accounts() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let first = Address::repeat_byte(0x01);
    let second = Address::repeat_byte(0x02);
    api.anvil_set_storage_at(first, U256::from(0), B256::with_last_byte(0xaa)).await.unwrap();
    api.anvil_set_storage_at(first, U256::from(1), B256::with_last_byte(0xbb)).await.unwrap();
    api.anvil_set_storage_at(second, U256::from(0), B256::with_last_byte(0xcc)).await.unwrap();

    let requests = HashMap::<Address, Vec<B256>>::from_iter([
        (first, vec![slot(0), slot(1), slot(2)]),
        (second, vec![slot(0)]),
    ]);
    let values: HashMap<Address, Vec<B256>> = provider
        .client()
        .request("eth_getStorageValues", (requests, BlockId::latest()))
        .await
        .unwrap();

    assert_eq!(
        values[&first],
        vec![B256::with_last_byte(0xaa), B256::with_last_byte(0xbb), B256::ZERO],
        "unset slots read as zero"
    );
    assert_eq!(values[&second], vec![B256::with_last_byte(0xcc)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_values_rejects_more_than_1024_slots() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let address = Address::repeat_byte(0x03);

    let at_limit = HashMap::<Address, Vec<B256>>::from_iter([(
        address,
        (0..1024).map(slot).collect::<Vec<_>>(),
    )]);
    let values: HashMap<Address, Vec<B256>> = provider
        .client()
        .request("eth_getStorageValues", (at_limit, BlockId::latest()))
        .await
        .unwrap();
    assert_eq!(values[&address].len(), 1024);

    // The limit is on the total slot count, not per account.
    let over_limit = HashMap::<Address, Vec<B256>>::from_iter([
        (address, (0..1024).map(slot).collect::<Vec<_>>()),
        (Address::repeat_byte(0x04), vec![slot(0)]),
    ]);
    let err = provider
        .client()
        .request::<_, HashMap<Address, Vec<B256>>>(
            "eth_getStorageValues",
            (over_limit, BlockId::latest()),
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("total slot count 1025 exceeds limit 1024"),
        "unexpected error: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_values_agrees_with_get_storage_at() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let address = Address::repeat_byte(0x05);

    api.anvil_set_storage_at(address, U256::from(0), B256::with_last_byte(0x11)).await.unwrap();
    api.mine_one().await.unwrap();
    let first = provider.get_block_number().await.unwrap();

    api.anvil_set_storage_at(address, U256::from(1), B256::with_last_byte(0x22)).await.unwrap();
    api.mine_one().await.unwrap();
    let second = provider.get_block_number().await.unwrap();

    // The batch endpoint must agree with the single-slot endpoint at every block it is asked
    // about, which is the property that matters for callers migrating between the two.
    let requests = HashMap::<Address, Vec<B256>>::from_iter([(address, vec![slot(0), slot(1)])]);
    for block in [first, second] {
        let batched: HashMap<Address, Vec<B256>> = provider
            .client()
            .request("eth_getStorageValues", (requests.clone(), BlockId::number(block)))
            .await
            .unwrap();

        let mut expected = Vec::new();
        for index in [U256::from(0), U256::from(1)] {
            let value = provider
                .get_storage_at(address, index)
                .block_id(BlockId::number(block))
                .await
                .unwrap();
            expected.push(B256::from(value));
        }
        assert_eq!(batched[&address], expected, "divergence at block {block}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_storage_values_skips_upstream_for_local_state() {
    let (_origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let (proxy, calls) = spawn_rpc_proxy_canned_method(
        origin_handle.http_endpoint(),
        "eth_getStorageAt",
        serde_json::json!(B256::with_last_byte(0xff)),
    )
    .await;

    let (api, handle) = spawn(NodeConfig::test().with_eth_rpc_url(Some(proxy))).await;
    let provider = handle.http_provider();
    let address = Address::repeat_byte(0x06);

    api.anvil_set_storage_at(address, U256::from(0), B256::with_last_byte(0x33)).await.unwrap();
    api.mine_one().await.unwrap();

    // State at or after the fork point is served locally, without querying the forked node.
    // Only the delta matters: spawning a fork legitimately reads storage for other reasons.
    let before = calls.load(Ordering::Relaxed);
    let requests = HashMap::<Address, Vec<B256>>::from_iter([(address, vec![slot(0)])]);
    let values: HashMap<Address, Vec<B256>> = provider
        .client()
        .request("eth_getStorageValues", (requests, BlockId::latest()))
        .await
        .unwrap();
    assert_eq!(values[&address], vec![B256::with_last_byte(0x33)]);
    assert_eq!(
        calls.load(Ordering::Relaxed),
        before,
        "post-fork state should not be fetched upstream"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_values_requires_an_explicit_block_parameter() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    // Like `eth_getStorageAt`, the trailing block parameter has to be present, even as `null`.
    let requests =
        HashMap::<Address, Vec<B256>>::from_iter([(Address::repeat_byte(0x07), vec![slot(0)])]);
    let err = provider
        .client()
        .request::<_, HashMap<Address, Vec<B256>>>("eth_getStorageValues", (requests.clone(),))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("expected tuple variant"), "unexpected error: {err}");

    let values: HashMap<Address, Vec<B256>> = provider
        .client()
        .request("eth_getStorageValues", (requests, Option::<BlockId>::None))
        .await
        .unwrap();
    assert_eq!(values[&Address::repeat_byte(0x07)], vec![B256::ZERO]);
}
