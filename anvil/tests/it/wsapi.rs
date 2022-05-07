//! general eth api tests with websocket provider

use crate::next_port;
use anvil::{spawn, NodeConfig};
use ethers::{prelude::Middleware, types::U256};

#[tokio::test(flavor = "multi_thread")]
async fn can_get_block_number_ws() {
    let (api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let block_num = api.block_number().unwrap();
    assert_eq!(block_num, U256::zero());

    let provider = handle.ws_provider().await;

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, block_num.as_u64().into());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_dev_get_balance_ws() {
    let (_api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.ws_provider().await;

    let genesis_balance = handle.genesis_balance();
    for acc in handle.genesis_accounts() {
        let balance = provider.get_balance(acc, None).await.unwrap();
        assert_eq!(balance, genesis_balance);
    }
}
