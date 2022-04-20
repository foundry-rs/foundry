//! general eth api tests

use crate::{init_tracing, next_port};
use anvil::{spawn, NodeConfig};
use ethers::{
    contract::abigen,
    middleware::SignerMiddleware,
    prelude::Middleware,
    types::{Filter, ValueOrArray},
};
use futures::StreamExt;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_new_heads() {
    let (api, handle) = spawn(NodeConfig::test().port(next_port())).await;

    let provider = handle.ws_provider().await;

    let blocks = provider.subscribe_blocks().await.unwrap();

    // mine a block every 1 seconds
    api.anvil_set_interval_mining(1).unwrap();

    let blocks = blocks.take(3).collect::<Vec<_>>().await;
    let block_numbers = blocks.into_iter().map(|b| b.number.unwrap().as_u64()).collect::<Vec<_>>();

    assert_eq!(block_numbers, vec![1, 2, 3]);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_logs() {
    init_tracing();
    abigen!(EmitLogs, "test-data/emit_logs.json");

    let (_api, handle) = spawn(NodeConfig::test().port(next_port())).await;
    let provider = handle.ws_provider().await;

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let msg = "First Message".to_string();
    let contract =
        EmitLogs::deploy(Arc::clone(&client), msg.clone()).unwrap().legacy().send().await.unwrap();

    let val = contract.get_value().call().await.unwrap();
    assert_eq!(val, msg);

    let val = contract
        .set_value("Next Message".to_string())
        .legacy()
        .send()
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    // let log = logs_sub.next().await.unwrap();
    //
    // dbg!(log);
    // // mine a block every 1 seconds
    // api.anvil_set_interval_mining(1).unwrap();
    //
    // let blocks = blocks.take(3).collect::<Vec<_>>().await;
    // let block_numbers = blocks.into_iter().map(|b|
    // b.number.unwrap().as_u64()).collect::<Vec<_>>();
    //
    // assert_eq!(block_numbers, vec![1, 2, 3]);
}
