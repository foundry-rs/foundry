//! various fork related test

use crate::{next_port, utils};
use anvil::{spawn, NodeConfig};
use anvil_core::types::Forking;
use ethers::{
    prelude::Middleware,
    signers::Signer,
    types::{Address, BlockNumber, Chain, TransactionRequest},
};

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

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let block_number = provider.get_block_number().await.unwrap();
    let balance_before = provider.get_balance(to, None).await.unwrap();
    let amount = handle.genesis_balance().checked_div(2u64.into()).unwrap();

    let tx = TransactionRequest::new().to(to).value(amount).from(from);

    let tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();
    assert_eq!(tx.transaction_index, 0u64.into());

    let nonce = provider.get_transaction_count(from, None).await.unwrap();

    assert_eq!(nonce, 1u64.into());
    let to_balance = provider.get_balance(to, None).await.unwrap();
    assert_eq!(balance_before.saturating_add(amount), to_balance);

    api.anvil_reset(Some(Forking {
        json_rpc_url: None,
        block_number: Some(block_number.as_u64()),
    }))
    .await
    .unwrap();

    // reset block number
    assert_eq!(block_number, provider.get_block_number().await.unwrap());

    let nonce = provider.get_transaction_count(from, None).await.unwrap();
    assert_eq!(nonce, 0u64.into());
    let balance = provider.get_balance(from, None).await.unwrap();
    assert_eq!(balance, handle.genesis_balance());
    let balance = provider.get_balance(to, None).await.unwrap();
    assert_eq!(balance, handle.genesis_balance());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_snapshotting() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    let snapshot = api.evm_snapshot().await.unwrap();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let block_number = provider.get_block_number().await.unwrap();

    let balance_before = provider.get_balance(to, None).await.unwrap();
    let amount = handle.genesis_balance().checked_div(2u64.into()).unwrap();

    let tx = TransactionRequest::new().to(to).value(amount).from(from);

    let _ = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let nonce = provider.get_transaction_count(from, None).await.unwrap();
    assert_eq!(nonce, 1u64.into());
    let to_balance = provider.get_balance(to, None).await.unwrap();
    assert_eq!(balance_before.saturating_add(amount), to_balance);

    assert!(api.evm_revert(snapshot).await.unwrap());

    let nonce = provider.get_transaction_count(from, None).await.unwrap();
    assert_eq!(nonce, 0u64.into());
    let balance = provider.get_balance(from, None).await.unwrap();
    assert_eq!(balance, handle.genesis_balance());
    let balance = provider.get_balance(to, None).await.unwrap();
    assert_eq!(balance, handle.genesis_balance());
    assert_eq!(block_number, provider.get_block_number().await.unwrap());
}

/// tests that the remote state and local state are kept separate.
/// changes don't make into the read only Database that holds the remote state, which is flushed to
/// a cache file.
#[tokio::test(flavor = "multi_thread")]
async fn test_separate_states() {
    let (api, handle) = spawn(fork_config().with_fork_block_number(Some(14723772u64))).await;
    let provider = handle.http_provider();

    let addr: Address = "000000000000000000000000000000000000dEaD".parse().unwrap();

    let remote_balance = provider.get_balance(addr, None).await.unwrap();
    assert_eq!(remote_balance, 12556104082473169733500u128.into());

    api.anvil_set_balance(addr, 1337u64.into()).await.unwrap();
    let balance = provider.get_balance(addr, None).await.unwrap();
    assert_eq!(balance, 1337u64.into());

    let fork = api.get_fork().unwrap();
    let fork_db = fork.database.read();
    let acc = fork_db.inner().db().accounts.read().get(&addr).cloned().unwrap();

    assert_eq!(acc.balance, remote_balance)
}
