//! general eth api tests

use crate::next_port;
use anvil::{eth::api::CLIENT_VERSION, spawn, NodeConfig};
use ethers::{prelude::Middleware, types::U256};

#[tokio::test(flavor = "multi_thread")]
async fn can_get_block_number() {
    let (api, handle) = spawn(NodeConfig::test().port(next_port())).await;

    let block_num = api.block_number().unwrap();
    assert_eq!(block_num, U256::zero());

    let provider = handle.http_provider();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, block_num.as_u64().into());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_dev_get_balance() {
    let (_api, handle) = spawn(NodeConfig::test().port(next_port())).await;
    let provider = handle.http_provider();

    let genesis_balance = handle.genesis_balance();
    for acc in handle.genesis_accounts() {
        let balance = provider.get_balance(acc, None).await.unwrap();
        assert_eq!(balance, genesis_balance);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_price() {
    let (_api, handle) = spawn(NodeConfig::test().port(next_port())).await;
    let provider = handle.http_provider();

    let _ = provider.get_gas_price().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_accounts() {
    let (_api, handle) = spawn(NodeConfig::test().port(next_port())).await;
    let provider = handle.http_provider();

    let _ = provider.get_accounts().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_client_version() {
    let (_api, handle) = spawn(NodeConfig::test().port(next_port())).await;
    let provider = handle.http_provider();

    let version = provider.client_version().await.unwrap();
    assert_eq!(CLIENT_VERSION, version);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_chain_id() {
    let (_api, handle) = spawn(NodeConfig::test().port(next_port())).await;
    let provider = handle.http_provider();

    let chain_id = provider.get_chainid().await.unwrap();
    assert_eq!(chain_id, 1337u64.into());
}
