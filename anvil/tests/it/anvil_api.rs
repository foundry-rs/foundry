//! tests for custom anvil endpoints

use crate::next_port;
use anvil::{spawn, Hardfork, NodeConfig};
use ethers::{
    prelude::Middleware,
    types::{Address, BlockNumber, TransactionRequest, U256},
};
use std::time::{Duration, SystemTime};

#[tokio::test(flavor = "multi_thread")]
async fn can_set_gas_price() {
    let (api, handle) =
        spawn(NodeConfig::test().with_port(next_port()).with_hardfork(Hardfork::Berlin)).await;
    let provider = handle.http_provider();

    let gas_price = 1337u64.into();
    api.anvil_set_min_gas_price(gas_price).await.unwrap();
    assert_eq!(gas_price, provider.get_gas_price().await.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_impersonate_account() {
    let (api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.http_provider();

    let impersonate = Address::random();
    let to = Address::random();
    let val = 1337u64;

    // fund the impersonated account
    api.anvil_set_balance(impersonate, U256::from(1e18 as u64)).await.unwrap();

    let tx = TransactionRequest::new().from(impersonate).to(to).value(val);

    let res = provider.send_transaction(tx.clone(), None).await;
    assert!(res.is_err());

    api.anvil_impersonate_account(impersonate).await.unwrap();

    let res = provider.send_transaction(tx.clone(), None).await.unwrap().await.unwrap().unwrap();
    assert_eq!(res.from, impersonate);

    let nonce = provider.get_transaction_count(impersonate, None).await.unwrap();
    assert_eq!(nonce, 1u64.into());

    let balance = provider.get_balance(to, None).await.unwrap();
    assert_eq!(balance, val.into());

    api.anvil_stop_impersonating_account(impersonate).await.unwrap();
    let res = provider.send_transaction(tx, None).await;
    assert!(res.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_mine_manually() {
    let (api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.http_provider();

    let start_num = provider.get_block_number().await.unwrap();

    for (idx, _) in std::iter::repeat(()).take(10).enumerate() {
        api.evm_mine(None).await.unwrap();
        let num = provider.get_block_number().await.unwrap();
        assert_eq!(num, start_num + idx + 1);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_set_next_timestamp() {
    let (api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.http_provider();

    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap();

    let next_timestamp = now + Duration::from_secs(60);

    // mock timestamp
    api.evm_set_next_block_timestamp(next_timestamp.as_secs()).unwrap();

    api.evm_mine(None).await.unwrap();

    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();

    assert_eq!(block.number.unwrap().as_u64(), 1);
    assert_eq!(block.timestamp.as_u64(), next_timestamp.as_secs());

    api.evm_mine(None).await.unwrap();

    let next = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();
    assert_eq!(next.number.unwrap().as_u64(), 2);

    assert!(next.timestamp > block.timestamp);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_timestamp_interval() {
    let (api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.http_provider();

    api.evm_mine(None).await.unwrap();
    let interval = 10;

    for _ in 0..5 {
        let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();

        // mock timestamp
        api.evm_set_block_timestamp_interval(interval).unwrap();
        api.evm_mine(None).await.unwrap();

        let new_block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();

        assert_eq!(new_block.timestamp, block.timestamp + interval);
    }

    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();

    let next_timestamp = block.timestamp + 50;
    api.evm_set_next_block_timestamp(next_timestamp.as_u64()).unwrap();

    api.evm_mine(None).await.unwrap();
    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();
    assert_eq!(block.timestamp, next_timestamp);

    api.evm_mine(None).await.unwrap();

    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();
    // interval also works after setting the next timestamp manually
    assert_eq!(block.timestamp, next_timestamp + interval);

    assert!(api.evm_remove_block_timestamp_interval().unwrap());

    api.evm_mine(None).await.unwrap();
    let new_block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();

    // offset is applied correctly after resetting the interval
    assert!(new_block.timestamp > block.timestamp);

    api.evm_mine(None).await.unwrap();
    let another_block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();
    // check interval is disabled
    assert!(another_block.timestamp - new_block.timestamp < U256::from(interval));
}
