//! various fork related test

use crate::{next_port, utils};
use anvil::{eth::EthApi, spawn, NodeConfig, NodeHandle};
use anvil_core::types::Forking;
use ethers::{
    contract::abigen,
    prelude::{Middleware, SignerMiddleware},
    signers::Signer,
    types::{Address, BlockNumber, Chain, TransactionRequest},
};
use std::sync::Arc;

// import helper module that provides rotating rpc endpoints
#[path = "../../../cli/test-utils/src/rpc.rs"]
mod rpc;

abigen!(Greeter, "test-data/greeter.json");

const BLOCK_NUMBER: u64 = 14_608_400u64;

const BLOCK_TIMESTAMP: u64 = 1_650_274_250u64;

/// Represents an anvil fork of an anvil node
#[allow(unused)]
pub struct LocalFork {
    origin_api: EthApi,
    origin_handle: NodeHandle,
    fork_api: EthApi,
    fork_handle: NodeHandle,
}

// === impl LocalFork ===
#[allow(dead_code)]
impl LocalFork {
    /// Spawns two nodes with the test config
    pub async fn new() -> Self {
        Self::setup(
            NodeConfig::test().with_port(next_port()),
            NodeConfig::test().with_port(next_port()),
        )
        .await
    }

    /// Spawns two nodes where one is a fork of the other
    pub async fn setup(origin: NodeConfig, fork: NodeConfig) -> Self {
        let (origin_api, origin_handle) = spawn(origin).await;

        let (fork_api, fork_handle) =
            spawn(fork.with_eth_rpc_url(Some(origin_handle.http_endpoint()))).await;
        Self { origin_api, origin_handle, fork_api, fork_handle }
    }
}

fn fork_config() -> NodeConfig {
    NodeConfig::test()
        .with_port(next_port())
        .with_eth_rpc_url(Some(rpc::next_http_archive_rpc_endpoint()))
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

    let initial_nonce = provider.get_transaction_count(from, None).await.unwrap();

    let tx = TransactionRequest::new().to(to).value(amount).from(from);

    let tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();
    assert_eq!(tx.transaction_index, 0u64.into());

    let nonce = provider.get_transaction_count(from, None).await.unwrap();

    assert_eq!(nonce, initial_nonce + 1);
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
    assert_eq!(nonce, initial_nonce);
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

    let initial_nonce = provider.get_transaction_count(from, None).await.unwrap();
    let balance_before = provider.get_balance(to, None).await.unwrap();
    let amount = handle.genesis_balance().checked_div(2u64.into()).unwrap();

    let tx = TransactionRequest::new().to(to).value(amount).from(from);

    let _ = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let nonce = provider.get_transaction_count(from, None).await.unwrap();
    assert_eq!(nonce, initial_nonce + 1);
    let to_balance = provider.get_balance(to, None).await.unwrap();
    assert_eq!(balance_before.saturating_add(amount), to_balance);

    assert!(api.evm_revert(snapshot).await.unwrap());

    let nonce = provider.get_transaction_count(from, None).await.unwrap();
    assert_eq!(nonce, initial_nonce);
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

#[tokio::test(flavor = "multi_thread")]
async fn can_deploy_greeter_on_fork() {
    let (_api, handle) = spawn(fork_config().with_fork_block_number(Some(14723772u64))).await;
    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let greeter_contract = Greeter::deploy(Arc::clone(&client), "Hello World!".to_string())
        .unwrap()
        .send()
        .await
        .unwrap();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);

    let greeter_contract =
        Greeter::deploy(client, "Hello World!".to_string()).unwrap().send().await.unwrap();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);
}

/// tests that we can deploy from dev account that already has an onchain presence: https://rinkeby.etherscan.io/address/0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266
#[tokio::test(flavor = "multi_thread")]
async fn can_deploy_greeter_on_rinkeby_fork() {
    let (_api, handle) = spawn(
        NodeConfig::test()
            .with_port(next_port())
            .with_eth_rpc_url(Some(rpc::next_rinkeby_http_rpc_endpoint()))
            .silent()
            .with_fork_block_number(Some(10074295u64)),
    )
    .await;
    let provider = handle.http_provider();
    let wallet = handle.dev_wallets().next().unwrap();
    let from = wallet.address();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));
    assert_eq!(client.get_transaction_count(from, None).await.unwrap(), 5845u64.into());

    let greeter_contract = Greeter::deploy(Arc::clone(&client), "Hello World!".to_string())
        .unwrap()
        .send()
        .await
        .unwrap();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);

    let greeter_contract =
        Greeter::deploy(client, "Hello World!".to_string()).unwrap().send().await.unwrap();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_reset_properly() {
    let (origin_api, origin_handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let account = origin_handle.dev_accounts().next().unwrap();
    let origin_provider = origin_handle.http_provider();
    let origin_nonce = 1u64.into();
    origin_api.anvil_set_nonce(account, origin_nonce).await.unwrap();

    assert_eq!(origin_nonce, origin_provider.get_transaction_count(account, None).await.unwrap());

    let (fork_api, fork_handle) = spawn(
        NodeConfig::test()
            .with_port(next_port())
            .with_eth_rpc_url(Some(origin_handle.http_endpoint())),
    )
    .await;

    let fork_provider = fork_handle.http_provider();
    assert_eq!(origin_nonce, fork_provider.get_transaction_count(account, None).await.unwrap());

    let to = Address::random();
    let to_balance = fork_provider.get_balance(to, None).await.unwrap();
    let tx = TransactionRequest::new().from(account).to(to).value(1337u64);
    let tx = fork_provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    // nonce incremented by 1
    assert_eq!(origin_nonce + 1, fork_provider.get_transaction_count(account, None).await.unwrap());

    // resetting to origin state
    fork_api.anvil_reset(Some(Forking::default())).await.unwrap();

    // nonce reset to origin
    assert_eq!(origin_nonce, fork_provider.get_transaction_count(account, None).await.unwrap());

    // balance is reset
    assert_eq!(to_balance, fork_provider.get_balance(to, None).await.unwrap());

    // tx does not exist anymore
    assert!(fork_provider.get_transaction(tx.transaction_hash).await.unwrap().is_none())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_timestamp() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    let block = provider.get_block(BLOCK_NUMBER).await.unwrap().unwrap();
    assert_eq!(block.timestamp.as_u64(), BLOCK_TIMESTAMP);

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();

    let tx = TransactionRequest::new().to(Address::random()).value(1337u64).from(from);
    let _tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();

    // ensure the diff between the new mined block and the original block is within a small window
    // to account for network delays, timestamp rounding: 3 secs and the http provider's
    // interval, just to be safe
    let expected_timestamp_offset = provider.get_interval().as_secs() + 3;
    let diff = block.timestamp - BLOCK_TIMESTAMP;
    assert!(diff <= expected_timestamp_offset.into());

    // reset to check timestamp works after resetting
    api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(BLOCK_NUMBER) }))
        .await
        .unwrap();
    let block = provider.get_block(BLOCK_NUMBER).await.unwrap().unwrap();
    assert_eq!(block.timestamp.as_u64(), BLOCK_TIMESTAMP);

    let tx = TransactionRequest::new().to(Address::random()).value(1337u64).from(from);
    let _tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();
    // ensure the diff between the new mined block and the original block is within 2secs, just to
    // be safe
    let diff = block.timestamp - BLOCK_TIMESTAMP;
    assert!(diff <= expected_timestamp_offset.into());
}
