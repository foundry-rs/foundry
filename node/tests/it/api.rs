//! general eth api tests

use crate::next_port;
use ethers::{
    prelude::{Http, Middleware, Provider},
    types::U256,
};
use foundry_node::{spawn, NodeConfig};

#[tokio::test(flavor = "multi_thread")]
async fn can_get_block_number() {
    let (api, handle) = spawn(NodeConfig::default().port(next_port()));

    let block_num = api.block_number().unwrap();
    assert_eq!(block_num, U256::zero());

    let provider = handle.http_provider();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, block_num.as_u64().into());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_dev_get_balance() {
    let (api, handle) = spawn(NodeConfig::default().port(next_port()));
    let provider = handle.http_provider();

    let genesis_balance = handle.genesis_balance();
    for acc in handle.genesis_accounts() {
        let balance = provider.get_balance(acc, None).await.unwrap();
        assert_eq!(balance, genesis_balance);
    }
}
