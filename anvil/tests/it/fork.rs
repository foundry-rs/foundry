//! various fork related test

use crate::{next_port, utils};
use anvil::{spawn, NodeConfig};
use ethers::{
    prelude::Middleware,
    types::{Address, BlockNumber, Chain},
};

#[allow(unused)]
use anvil::init_tracing;

const RPC_RPC_URL: &str = "https://eth-mainnet.alchemyapi.io/v2/Lc7oIGYeL_QvInzI0Wiu_pOZZDEKBrdf";

const BLOCK_NUMBER: u64 = 14_608_400u64;

fn fork_config() -> NodeConfig {
    NodeConfig::test()
        .with_port(next_port())
        .with_eth_rpc_url(Some(RPC_RPC_URL))
        .with_fork_block_number(Some(BLOCK_NUMBER))
        .silent()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_spawn_fork() {
    let (api, _handle) = spawn(fork_config()).await;
    assert!(api.is_fork());

    let head = api.block_number().unwrap();
    assert_eq!(head, BLOCK_NUMBER.into())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_eth_get_balance() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();
    for _ in 0..10 {
        let addr = Address::random();
        let balance = api.balance(addr, None).await.unwrap();
        let provider_balance = provider.get_balance(addr, None).await.unwrap();
        assert_eq!(balance, provider_balance)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_eth_get_code() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();
    for _ in 0..10 {
        let addr = Address::random();
        let code = api.get_code(addr, None).await.unwrap();
        let provider_code = provider.get_code(addr, None).await.unwrap();
        assert_eq!(code, provider_code)
    }

    for address in utils::contract_addresses(Chain::Mainnet) {
        let code = api.get_code(address, None).await.unwrap();
        let provider_code = provider.get_code(address, None).await.unwrap();
        assert_eq!(code, provider_code);
        assert!(!code.as_ref().is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_eth_get_nonce() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    for _ in 0..10 {
        let addr = Address::random();
        let api_nonce = api.transaction_count(addr, None).await.unwrap();
        let provider_nonce = provider.get_transaction_count(addr, None).await.unwrap();
        assert_eq!(api_nonce, provider_nonce);
    }

    let addr: Address = "0x00a329c0648769a73afac7f9381e08fb43dbea72".parse().unwrap();
    let api_nonce = api.transaction_count(addr, None).await.unwrap();
    let provider_nonce = provider.get_transaction_count(addr, None).await.unwrap();
    assert_eq!(api_nonce, provider_nonce);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_eth_fee_history() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    let count = 10u64;
    let _history = api.fee_history(count.into(), BlockNumber::Latest, vec![]).unwrap();
    let _provider_history = provider.fee_history(count, BlockNumber::Latest, &[]).await.unwrap();
}
