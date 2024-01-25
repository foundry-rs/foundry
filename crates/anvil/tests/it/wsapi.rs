//! general eth api tests with websocket provider

use alloy_providers::provider::TempProvider;
use anvil::{spawn, NodeConfig};
use ethers::types::U256;
use foundry_common::types::ToAlloy;

#[tokio::test(flavor = "multi_thread")]
async fn can_get_block_number_ws() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let block_num = api.block_number().unwrap();
    assert_eq!(block_num, U256::zero().to_alloy());

    let provider = handle.ws_provider();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, block_num.to::<u64>());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_dev_get_balance_ws() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ws_provider();

    let genesis_balance = handle.genesis_balance();
    for acc in handle.genesis_accounts() {
        let balance = provider.get_balance(acc, None).await.unwrap();
        assert_eq!(balance, genesis_balance);
    }
}
