//! Gas related tests

use anvil::{eth::fees::INITIAL_BASE_FEE, spawn, NodeConfig};
use ethers::{
    prelude::Middleware,
    types::{Address, BlockNumber, TransactionRequest},
};

const GAS_TRANSFER: u64 = 21_000u64;

#[tokio::test(flavor = "multi_thread")]
async fn test_basefee_full_block() {
    let (_api, handle) = spawn(
        NodeConfig::test().with_base_fee(Some(INITIAL_BASE_FEE)).with_gas_limit(Some(GAS_TRANSFER)),
    )
    .await;
    let provider = handle.http_provider();
    let tx = TransactionRequest::new().to(Address::random()).value(1337u64);
    provider.send_transaction(tx.clone(), None).await.unwrap().await.unwrap().unwrap();
    let base_fee =
        provider.get_block(BlockNumber::Latest).await.unwrap().unwrap().base_fee_per_gas.unwrap();
    let tx = TransactionRequest::new().to(Address::random()).value(1337u64);
    provider.send_transaction(tx.clone(), None).await.unwrap().await.unwrap().unwrap();
    let next_base_fee =
        provider.get_block(BlockNumber::Latest).await.unwrap().unwrap().base_fee_per_gas.unwrap();

    assert!(next_base_fee > base_fee);
    // max increase, full block
    assert_eq!(next_base_fee.as_u64(), INITIAL_BASE_FEE + 125_000_000);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_basefee_half_block() {
    let (_api, handle) = spawn(
        NodeConfig::test()
            .with_base_fee(Some(INITIAL_BASE_FEE))
            .with_gas_limit(Some(GAS_TRANSFER * 2)),
    )
    .await;
    let provider = handle.http_provider();
    let tx = TransactionRequest::new().to(Address::random()).value(1337u64);
    provider.send_transaction(tx.clone(), None).await.unwrap().await.unwrap().unwrap();
    let tx = TransactionRequest::new().to(Address::random()).value(1337u64);
    provider.send_transaction(tx.clone(), None).await.unwrap().await.unwrap().unwrap();
    let next_base_fee =
        provider.get_block(BlockNumber::Latest).await.unwrap().unwrap().base_fee_per_gas.unwrap();

    // unchanged, half block
    assert_eq!(next_base_fee.as_u64(), INITIAL_BASE_FEE);
}
#[tokio::test(flavor = "multi_thread")]
async fn test_basefee_empty_block() {
    let (api, handle) = spawn(NodeConfig::test().with_base_fee(Some(INITIAL_BASE_FEE))).await;

    let provider = handle.http_provider();
    let tx = TransactionRequest::new().to(Address::random()).value(1337u64);
    provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();
    let base_fee =
        provider.get_block(BlockNumber::Latest).await.unwrap().unwrap().base_fee_per_gas.unwrap();

    // mine empty block
    api.mine_one().await;

    let next_base_fee =
        provider.get_block(BlockNumber::Latest).await.unwrap().unwrap().base_fee_per_gas.unwrap();

    // empty block, decreased base fee
    assert!(next_base_fee < base_fee);
}
