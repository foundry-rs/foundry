//! various fork related test

use crate::{abi::*, utils};
use alloy_primitives::U256 as rU256;
use alloy_rpc_types::{BlockNumberOrTag, CallRequest};
use anvil::{eth::EthApi, spawn, NodeConfig, NodeHandle};
use anvil_core::types::Forking;
use ethers::{
    core::rand,
    prelude::{Bytes, LocalWallet, Middleware, SignerMiddleware},
    providers::{Http, Provider},
    signers::Signer,
    types::{
        transaction::eip2718::TypedTransaction, Address, BlockNumber, Chain, TransactionRequest,
        U256,
    },
};
use foundry_common::{
    provider::ethers::get_http_provider,
    rpc,
    rpc::next_http_rpc_endpoint,
    types::{ToAlloy, ToEthers},
};
use foundry_config::Config;
use futures::StreamExt;
use std::{sync::Arc, time::Duration};

const BLOCK_NUMBER: u64 = 14_608_400u64;
const DEAD_BALANCE_AT_BLOCK_NUMBER: u128 = 12_556_069_338_441_120_059_867u128;

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
        Self::setup(NodeConfig::test(), NodeConfig::test()).await
    }

    /// Spawns two nodes where one is a fork of the other
    pub async fn setup(origin: NodeConfig, fork: NodeConfig) -> Self {
        let (origin_api, origin_handle) = spawn(origin).await;

        let (fork_api, fork_handle) =
            spawn(fork.with_eth_rpc_url(Some(origin_handle.http_endpoint()))).await;
        Self { origin_api, origin_handle, fork_api, fork_handle }
    }
}

pub fn fork_config() -> NodeConfig {
    NodeConfig::test()
        .with_eth_rpc_url(Some(rpc::next_http_archive_rpc_endpoint()))
        .with_fork_block_number(Some(BLOCK_NUMBER))
        .silent()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_spawn_fork() {
    let (api, _handle) = spawn(fork_config()).await;
    assert!(api.is_fork());

    let head = api.block_number().unwrap();
    assert_eq!(head, rU256::from(BLOCK_NUMBER))
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_eth_get_balance() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.ethers_http_provider();
    for _ in 0..10 {
        let addr = Address::random();
        let balance = api.balance(addr.to_alloy(), None).await.unwrap();
        let provider_balance = provider.get_balance(addr, None).await.unwrap();
        assert_eq!(balance, provider_balance.to_alloy())
    }
}

// <https://github.com/foundry-rs/foundry/issues/4082>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_eth_get_balance_after_mine() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.ethers_http_provider();
    let info = api.anvil_node_info().await.unwrap();
    let number = info.fork_config.fork_block_number.unwrap();
    assert_eq!(number, BLOCK_NUMBER);

    let address = Address::random();

    let _balance = provider
        .get_balance(address, Some(BlockNumber::Number(number.into()).into()))
        .await
        .unwrap();

    api.evm_mine(None).await.unwrap();

    let _balance = provider
        .get_balance(address, Some(BlockNumber::Number(number.into()).into()))
        .await
        .unwrap();
}

// <https://github.com/foundry-rs/foundry/issues/4082>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_eth_get_code_after_mine() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.ethers_http_provider();
    let info = api.anvil_node_info().await.unwrap();
    let number = info.fork_config.fork_block_number.unwrap();
    assert_eq!(number, BLOCK_NUMBER);

    let address = Address::random();

    let _code =
        provider.get_code(address, Some(BlockNumber::Number(number.into()).into())).await.unwrap();

    api.evm_mine(None).await.unwrap();

    let _code =
        provider.get_code(address, Some(BlockNumber::Number(number.into()).into())).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_eth_get_code() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.ethers_http_provider();
    for _ in 0..10 {
        let addr = Address::random();
        let code = api.get_code(addr.to_alloy(), None).await.unwrap();
        let provider_code = provider.get_code(addr, None).await.unwrap();
        assert_eq!(code, provider_code.to_alloy())
    }

    for address in utils::contract_addresses(Chain::Mainnet) {
        let prev_code = api
            .get_code(address.to_alloy(), Some(BlockNumberOrTag::Number(BLOCK_NUMBER - 10).into()))
            .await
            .unwrap();
        let code = api.get_code(address.to_alloy(), None).await.unwrap();
        let provider_code = provider.get_code(address, None).await.unwrap();
        assert_eq!(code, prev_code);
        assert_eq!(code, provider_code.to_alloy());
        assert!(!code.as_ref().is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_eth_get_nonce() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.ethers_http_provider();

    for _ in 0..10 {
        let addr = Address::random();
        let api_nonce = api.transaction_count(addr.to_alloy(), None).await.unwrap();
        let provider_nonce = provider.get_transaction_count(addr, None).await.unwrap();
        assert_eq!(api_nonce, provider_nonce.to_alloy());
    }

    let addr = Config::DEFAULT_SENDER;
    let api_nonce = api.transaction_count(addr, None).await.unwrap();
    let provider_nonce = provider.get_transaction_count(addr.to_ethers(), None).await.unwrap();
    assert_eq!(api_nonce, provider_nonce.to_alloy());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_eth_fee_history() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.ethers_http_provider();

    let count = 10u64;
    let _history =
        api.fee_history(rU256::from(count), BlockNumberOrTag::Latest, vec![]).await.unwrap();
    let _provider_history = provider.fee_history(count, BlockNumber::Latest, &[]).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.ethers_http_provider();

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

    // reset to latest
    api.anvil_reset(Some(Forking::default())).await.unwrap();

    let new_block_num = provider.get_block_number().await.unwrap();
    assert!(new_block_num > block_number);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_setup() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_http_provider();

    let dead_addr: Address = "000000000000000000000000000000000000dEaD".parse().unwrap();

    let block_number = provider.get_block_number().await.unwrap();
    assert_eq!(block_number, 0.into());

    let local_balance = provider.get_balance(dead_addr, None).await.unwrap();
    assert_eq!(local_balance, 0.into());

    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(rpc::next_http_archive_rpc_endpoint()),
        block_number: Some(BLOCK_NUMBER),
    }))
    .await
    .unwrap();

    let block_number = provider.get_block_number().await.unwrap();
    assert_eq!(block_number, BLOCK_NUMBER.into());

    let remote_balance = provider.get_balance(dead_addr, None).await.unwrap();
    assert_eq!(remote_balance, DEAD_BALANCE_AT_BLOCK_NUMBER.into());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_snapshotting() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.ethers_http_provider();

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
    let provider = handle.ethers_http_provider();

    let addr: Address = "000000000000000000000000000000000000dEaD".parse().unwrap();

    let remote_balance = provider.get_balance(addr, None).await.unwrap();
    assert_eq!(remote_balance, 12556104082473169733500u128.into());

    api.anvil_set_balance(addr.to_alloy(), rU256::from(1337u64)).await.unwrap();
    let balance = provider.get_balance(addr, None).await.unwrap();
    assert_eq!(balance, 1337u64.into());

    let fork = api.get_fork().unwrap();
    let fork_db = fork.database.read().await;
    let acc = fork_db
        .maybe_inner()
        .expect("could not get fork db inner")
        .db()
        .accounts
        .read()
        .get(&addr.to_alloy())
        .cloned()
        .unwrap();

    assert_eq!(acc.balance, remote_balance.to_alloy())
}

#[tokio::test(flavor = "multi_thread")]
async fn can_deploy_greeter_on_fork() {
    let (_api, handle) = spawn(fork_config().with_fork_block_number(Some(14723772u64))).await;
    let provider = handle.ethers_http_provider();

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

#[tokio::test(flavor = "multi_thread")]
async fn can_reset_properly() {
    let (origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let account = origin_handle.dev_accounts().next().unwrap();
    let origin_provider = origin_handle.ethers_http_provider();
    let origin_nonce = rU256::from(1u64);
    origin_api.anvil_set_nonce(account.to_alloy(), origin_nonce).await.unwrap();

    assert_eq!(
        origin_nonce,
        origin_provider.get_transaction_count(account, None).await.unwrap().to_alloy()
    );

    let (fork_api, fork_handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(origin_handle.http_endpoint()))).await;

    let fork_provider = fork_handle.ethers_http_provider();
    assert_eq!(
        origin_nonce,
        fork_provider.get_transaction_count(account, None).await.unwrap().to_alloy()
    );

    let to = Address::random();
    let to_balance = fork_provider.get_balance(to, None).await.unwrap();
    let tx = TransactionRequest::new().from(account).to(to).value(1337u64);
    let tx = fork_provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    // nonce incremented by 1
    assert_eq!(
        origin_nonce + rU256::from(1),
        fork_provider.get_transaction_count(account, None).await.unwrap().to_alloy()
    );

    // resetting to origin state
    fork_api.anvil_reset(Some(Forking::default())).await.unwrap();

    // nonce reset to origin
    assert_eq!(
        origin_nonce,
        fork_provider.get_transaction_count(account, None).await.unwrap().to_alloy()
    );

    // balance is reset
    assert_eq!(to_balance, fork_provider.get_balance(to, None).await.unwrap());

    // tx does not exist anymore
    assert!(fork_provider.get_transaction(tx.transaction_hash).await.is_err())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_timestamp() {
    let start = std::time::Instant::now();

    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.ethers_http_provider();

    let block = provider.get_block(BLOCK_NUMBER).await.unwrap().unwrap();
    assert_eq!(block.timestamp.as_u64(), BLOCK_TIMESTAMP);

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();

    let tx = TransactionRequest::new().to(Address::random()).value(1337u64).from(from);
    let tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();
    assert_eq!(tx.status, Some(1u64.into()));

    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();

    let elapsed = start.elapsed().as_secs() + 1;

    // ensure the diff between the new mined block and the original block is within the elapsed time
    let diff = block.timestamp - BLOCK_TIMESTAMP;
    assert!(diff <= elapsed.into(), "diff={diff}, elapsed={elapsed}");

    let start = std::time::Instant::now();
    // reset to check timestamp works after resetting
    api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(BLOCK_NUMBER) }))
        .await
        .unwrap();
    let block = provider.get_block(BLOCK_NUMBER).await.unwrap().unwrap();
    assert_eq!(block.timestamp.as_u64(), BLOCK_TIMESTAMP);

    let tx = TransactionRequest::new().to(Address::random()).value(1337u64).from(from);
    let _tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();
    let elapsed = start.elapsed().as_secs() + 1;
    let diff = block.timestamp - BLOCK_TIMESTAMP;
    assert!(diff <= elapsed.into());

    // ensure that after setting a timestamp manually, then next block time is correct
    let start = std::time::Instant::now();
    api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(BLOCK_NUMBER) }))
        .await
        .unwrap();
    api.evm_set_next_block_timestamp(BLOCK_TIMESTAMP + 1).unwrap();
    let tx = TransactionRequest::new().to(Address::random()).value(1337u64).from(from);
    let _tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();
    assert_eq!(block.timestamp.as_u64(), BLOCK_TIMESTAMP + 1);

    let tx = TransactionRequest::new().to(Address::random()).value(1337u64).from(from);
    let _tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();
    let elapsed = start.elapsed().as_secs() + 1;
    let diff = block.timestamp - (BLOCK_TIMESTAMP + 1);
    assert!(diff <= elapsed.into());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_set_empty_code() {
    let (api, _handle) = spawn(fork_config()).await;
    let addr = "0x1f9840a85d5af5bf1d1762f925bdaddc4201f984".parse().unwrap();
    let code = api.get_code(addr, None).await.unwrap();
    assert!(!code.as_ref().is_empty());
    api.anvil_set_code(addr, Vec::new().into()).await.unwrap();
    let code = api.get_code(addr, None).await.unwrap();
    assert!(code.as_ref().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_can_send_tx() {
    let (api, handle) =
        spawn(fork_config().with_blocktime(Some(std::time::Duration::from_millis(800)))).await;

    let wallet = LocalWallet::new(&mut rand::thread_rng());

    api.anvil_set_balance(wallet.address().to_alloy(), rU256::from(1e18 as u64)).await.unwrap();

    let provider = SignerMiddleware::new(handle.ethers_http_provider(), wallet);

    let addr = Address::random();
    let val = 1337u64;
    let tx = TransactionRequest::new().to(addr).value(val);

    // broadcast it via the eth_sendTransaction API
    let _ = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let balance = provider.get_balance(addr, None).await.unwrap();
    assert_eq!(balance, val.into());
}

// <https://github.com/foundry-rs/foundry/issues/1920>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_nft_set_approve_all() {
    let (api, handle) = spawn(
        fork_config()
            .with_fork_block_number(Some(14812197u64))
            .with_blocktime(Some(Duration::from_secs(5)))
            .with_chain_id(1u64.into()),
    )
    .await;

    // create and fund a random wallet
    let wallet = LocalWallet::new(&mut rand::thread_rng());
    api.anvil_set_balance(wallet.address().to_alloy(), rU256::from(1000e18 as u64)).await.unwrap();

    let provider = Arc::new(SignerMiddleware::new(handle.ethers_http_provider(), wallet.clone()));

    // pick a random nft <https://opensea.io/assets/ethereum/0x9c8ff314c9bc7f6e59a9d9225fb22946427edc03/154>
    let nouns_addr: Address = "0x9c8ff314c9bc7f6e59a9d9225fb22946427edc03".parse().unwrap();

    let owner: Address = "0x052564eb0fd8b340803df55def89c25c432f43f4".parse().unwrap();
    let token_id: U256 = 154u64.into();

    let nouns = Erc721::new(nouns_addr, Arc::clone(&provider));

    let real_owner = nouns.owner_of(token_id).call().await.unwrap();
    assert_eq!(real_owner, owner);
    let approval = nouns.set_approval_for_all(nouns_addr, true);
    let tx = approval.send().await.unwrap().await.unwrap().unwrap();
    assert_eq!(tx.status, Some(1u64.into()));

    let real_owner = real_owner.to_alloy();

    // transfer: impersonate real owner and transfer nft
    api.anvil_impersonate_account(real_owner).await.unwrap();

    api.anvil_set_balance(real_owner, rU256::from(10000e18 as u64)).await.unwrap();

    let call = nouns.transfer_from(real_owner.to_ethers(), wallet.address(), token_id);
    let mut tx: TypedTransaction = call.tx;
    tx.set_from(real_owner.to_ethers());
    provider.fill_transaction(&mut tx, None).await.unwrap();
    let tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();
    assert_eq!(tx.status, Some(1u64.into()));

    let real_owner = nouns.owner_of(token_id).call().await.unwrap();
    assert_eq!(real_owner, wallet.address());
}

// <https://github.com/foundry-rs/foundry/issues/2261>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_with_custom_chain_id() {
    // spawn a forked node with some random chainId
    let (api, handle) = spawn(
        fork_config()
            .with_fork_block_number(Some(14812197u64))
            .with_blocktime(Some(Duration::from_secs(5)))
            .with_chain_id(3145u64.into()),
    )
    .await;

    // get the eth chainId and the txn chainId
    let eth_chain_id = api.eth_chain_id();
    let txn_chain_id = api.chain_id();

    // get the chainId in the config
    let config_chain_id = handle.config().chain_id;

    // check that the chainIds are the same
    assert_eq!(eth_chain_id.unwrap().unwrap().to::<u64>(), 3145u64);
    assert_eq!(txn_chain_id, 3145u64);
    assert_eq!(config_chain_id, Some(3145u64));
}

// <https://github.com/foundry-rs/foundry/issues/1920>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_can_send_opensea_tx() {
    let (api, handle) = spawn(
        fork_config()
            .with_fork_block_number(Some(14983338u64))
            .with_blocktime(Some(Duration::from_millis(5000))),
    )
    .await;

    let sender: Address = "0x8fdbae54b6d9f3fc2c649e3dd4602961967fd42f".parse().unwrap();

    // transfer: impersonate real sender
    api.anvil_impersonate_account(sender.to_alloy()).await.unwrap();

    let provider = handle.ethers_http_provider();

    let input: Bytes = "0xfb0f3ee1000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000003ff2e795f5000000000000000000000000000023f28ae3e9756ba982a6290f9081b6a84900b758000000000000000000000000004c00500000ad104d7dbd00e3ae0a5c00560c0000000000000000000000000003235b597a78eabcb08ffcb4d97411073211dbcb0000000000000000000000000000000000000000000000000000000000000e72000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000062ad47c20000000000000000000000000000000000000000000000000000000062d43104000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000df44e65d2a2cf40000007b02230091a7ed01230072f7006a004d60a8d4e71d599b8104250f00000000007b02230091a7ed01230072f7006a004d60a8d4e71d599b8104250f00000000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000024000000000000000000000000000000000000000000000000000000000000002e000000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000001c6bf526340000000000000000000000000008de9c5a032463c561423387a9648c5c7bcc5bc900000000000000000000000000000000000000000000000000005543df729c0000000000000000000000000006eb234847a9e3a546539aac57a071c01dc3f398600000000000000000000000000000000000000000000000000000000000000416d39b5352353a22cf2d44faa696c2089b03137a13b5acfee0366306f2678fede043bc8c7e422f6f13a3453295a4a063dac7ee6216ab7bade299690afc77397a51c00000000000000000000000000000000000000000000000000000000000000".parse().unwrap();
    let to: Address = "0x00000000006c3852cbef3e08e8df289169ede581".parse().unwrap();
    let tx = TransactionRequest::new()
        .from(sender)
        .to(to)
        .value(20000000000000000u64)
        .data(input)
        .gas_price(22180711707u64)
        .gas(150_000u64);

    let tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();
    assert_eq!(tx.status, Some(1u64.into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_base_fee() {
    let (api, handle) = spawn(fork_config()).await;

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();

    let provider = handle.ethers_http_provider();

    api.anvil_set_next_block_base_fee_per_gas(rU256::ZERO).await.unwrap();

    let addr = Address::random();
    let val = 1337u64;
    let tx = TransactionRequest::new().from(from).to(addr).value(val);

    let _res = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_init_base_fee() {
    let (api, handle) = spawn(fork_config().with_fork_block_number(Some(13184859u64))).await;

    let provider = handle.ethers_http_provider();

    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();
    // <https://etherscan.io/block/13184859>
    assert_eq!(block.number.unwrap().as_u64(), 13184859u64);
    let init_base_fee = block.base_fee_per_gas.unwrap();
    assert_eq!(init_base_fee, 63739886069u64.into());

    api.mine_one().await;

    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();

    let next_base_fee = block.base_fee_per_gas.unwrap();
    assert!(next_base_fee < init_base_fee);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reset_fork_on_new_blocks() {
    let (api, handle) = spawn(
        NodeConfig::test().with_eth_rpc_url(Some(rpc::next_http_archive_rpc_endpoint())).silent(),
    )
    .await;

    let anvil_provider = handle.ethers_http_provider();

    let endpoint = next_http_rpc_endpoint();
    let provider = Arc::new(get_http_provider(&endpoint).interval(Duration::from_secs(2)));

    let current_block = anvil_provider.get_block_number().await.unwrap();

    handle.task_manager().spawn_reset_on_new_polled_blocks(provider.clone(), api);

    let mut stream = provider.watch_blocks().await.unwrap();
    // the http watcher may fetch multiple blocks at once, so we set a timeout here to offset edge
    // cases where the stream immediately returns a block
    tokio::time::sleep(Chain::Mainnet.average_blocktime_hint().unwrap()).await;
    stream.next().await.unwrap();
    stream.next().await.unwrap();

    let next_block = anvil_provider.get_block_number().await.unwrap();

    assert!(next_block > current_block, "nextblock={next_block} currentblock={current_block}")
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_call() {
    let input: Bytes = "0x77c7b8fc".parse().unwrap();
    let to: Address = "0x99d1Fa417f94dcD62BfE781a1213c092a47041Bc".parse().unwrap();
    let block_number = 14746300u64;

    let provider = Provider::<Http>::try_from(rpc::next_http_archive_rpc_endpoint()).unwrap();
    let mut tx = TypedTransaction::default();
    tx.set_to(to).set_data(input.clone());
    let res0 =
        provider.call(&tx, Some(BlockNumber::Number(block_number.into()).into())).await.unwrap();

    let (api, _) = spawn(fork_config().with_fork_block_number(Some(block_number))).await;

    let res1 = api
        .call(
            CallRequest {
                to: Some(to.to_alloy()),
                input: input.to_alloy().into(),
                ..Default::default()
            },
            None,
            None,
        )
        .await
        .unwrap();

    assert_eq!(res0, res1.to_ethers());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_block_timestamp() {
    let (api, _) = spawn(fork_config()).await;

    let initial_block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    api.anvil_mine(Some(rU256::from(1)), None).await.unwrap();
    let latest_block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();

    assert!(initial_block.header.timestamp.to::<u64>() < latest_block.header.timestamp.to::<u64>());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_snapshot_block_timestamp() {
    let (api, _) = spawn(fork_config()).await;

    let snapshot_id = api.evm_snapshot().await.unwrap();
    api.anvil_mine(Some(rU256::from(1)), None).await.unwrap();
    let initial_block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    api.evm_revert(snapshot_id).await.unwrap();
    api.evm_set_next_block_timestamp(initial_block.header.timestamp.to::<u64>()).unwrap();
    api.anvil_mine(Some(rU256::from(1)), None).await.unwrap();
    let latest_block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();

    assert_eq!(
        initial_block.header.timestamp.to::<u64>(),
        latest_block.header.timestamp.to::<u64>()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_uncles_fetch() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.ethers_http_provider();

    // Block on ETH mainnet with 2 uncles
    let block_with_uncles = 190u64;

    let block =
        api.block_by_number(BlockNumberOrTag::Number(block_with_uncles)).await.unwrap().unwrap();

    assert_eq!(block.uncles.len(), 2);

    let count = provider.get_uncle_count(block_with_uncles).await.unwrap();
    assert_eq!(count.as_usize(), block.uncles.len());

    let count = provider.get_uncle_count(block.header.hash.unwrap().to_ethers()).await.unwrap();
    assert_eq!(count.as_usize(), block.uncles.len());

    for (uncle_idx, uncle_hash) in block.uncles.iter().enumerate() {
        // Try with block number
        let uncle = provider
            .get_uncle(block_with_uncles, (uncle_idx as u64).into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(*uncle_hash, uncle.hash.unwrap().to_alloy());

        // Try with block hash
        let uncle = provider
            .get_uncle(block.header.hash.unwrap().to_ethers(), (uncle_idx as u64).into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(*uncle_hash, uncle.hash.unwrap().to_alloy());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_block_transaction_count() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.ethers_http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let sender = accounts[0].address();

    // disable automine (so there are pending transactions)
    api.anvil_set_auto_mine(false).await.unwrap();
    // transfer: impersonate real sender
    api.anvil_impersonate_account(sender.to_alloy()).await.unwrap();

    let tx = TransactionRequest::new().from(sender).value(42u64).gas(100_000);
    provider.send_transaction(tx, None).await.unwrap();

    let pending_txs =
        api.block_transaction_count_by_number(BlockNumberOrTag::Pending).await.unwrap().unwrap();
    assert_eq!(pending_txs.to::<u64>(), 1);

    // mine a new block
    api.anvil_mine(None, None).await.unwrap();

    let pending_txs =
        api.block_transaction_count_by_number(BlockNumberOrTag::Pending).await.unwrap().unwrap();
    assert_eq!(pending_txs.to::<u64>(), 0);
    let latest_txs =
        api.block_transaction_count_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert_eq!(latest_txs.to::<u64>(), 1);
    let latest_block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    let latest_txs = api
        .block_transaction_count_by_hash(latest_block.header.hash.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest_txs.to::<u64>(), 1);

    // check txs count on an older block: 420000 has 3 txs on mainnet
    let count_txs = api
        .block_transaction_count_by_number(BlockNumberOrTag::Number(420000))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(count_txs.to::<u64>(), 3);
    let count_txs = api
        .block_transaction_count_by_hash(
            "0xb3b0e3e0c64e23fb7f1ccfd29245ae423d2f6f1b269b63b70ff882a983ce317c".parse().unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(count_txs.to::<u64>(), 3);
}

// <https://github.com/foundry-rs/foundry/issues/2931>
#[tokio::test(flavor = "multi_thread")]
async fn can_impersonate_in_fork() {
    let (api, handle) = spawn(fork_config().with_fork_block_number(Some(15347924u64))).await;
    let provider = handle.ethers_http_provider();

    let token_holder: Address = "0x2f0b23f53734252bda2277357e97e1517d6b042a".parse().unwrap();
    let to = Address::random();
    let val = 1337u64;

    // fund the impersonated account
    api.anvil_set_balance(token_holder.to_alloy(), rU256::from(1e18 as u64)).await.unwrap();

    let tx = TransactionRequest::new().from(token_holder).to(to).value(val);

    let res = provider.send_transaction(tx.clone(), None).await;
    res.unwrap_err();

    api.anvil_impersonate_account(token_holder.to_alloy()).await.unwrap();

    let res = provider.send_transaction(tx.clone(), None).await.unwrap().await.unwrap().unwrap();
    assert_eq!(res.from, token_holder);
    assert_eq!(res.status, Some(1u64.into()));

    let balance = provider.get_balance(to, None).await.unwrap();
    assert_eq!(balance, val.into());

    api.anvil_stop_impersonating_account(token_holder.to_alloy()).await.unwrap();
    let res = provider.send_transaction(tx, None).await;
    res.unwrap_err();
}

// <https://etherscan.io/block/14608400>
#[tokio::test(flavor = "multi_thread")]
async fn test_total_difficulty_fork() {
    let (api, handle) = spawn(fork_config()).await;

    let total_difficulty: U256 = 46_673_965_560_973_856_260_636u128.into();
    let difficulty: U256 = 13_680_435_288_526_144u128.into();

    let provider = handle.ethers_http_provider();
    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();
    assert_eq!(block.total_difficulty, Some(total_difficulty));
    assert_eq!(block.difficulty, difficulty);

    api.mine_one().await;
    api.mine_one().await;

    let next_total_difficulty = total_difficulty + difficulty;

    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();
    assert_eq!(block.total_difficulty, Some(next_total_difficulty));
    assert_eq!(block.difficulty, U256::zero());
}

// <https://etherscan.io/block/14608400>
#[tokio::test(flavor = "multi_thread")]
async fn test_transaction_receipt() {
    let (api, _) = spawn(fork_config()).await;

    // A transaction from the forked block (14608400)
    let receipt = api
        .transaction_receipt(
            "0xce495d665e9091613fd962351a5cbca27a992b919d6a87d542af97e2723ec1e4".parse().unwrap(),
        )
        .await
        .unwrap();
    assert!(receipt.is_some());

    // A transaction from a block in the future (14608401)
    let receipt = api
        .transaction_receipt(
            "0x1a15472088a4a97f29f2f9159511dbf89954b58d9816e58a32b8dc17171dc0e8".parse().unwrap(),
        )
        .await
        .unwrap();
    assert!(receipt.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_override_fork_chain_id() {
    let chain_id_override = 5u64;
    let (_api, handle) = spawn(
        fork_config()
            .with_fork_block_number(Some(16506610u64))
            .with_chain_id(Some(chain_id_override)),
    )
    .await;
    let provider = handle.ethers_http_provider();

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

    let provider = handle.ethers_http_provider();
    let chain_id = provider.get_chainid().await.unwrap();
    assert_eq!(chain_id.as_u64(), chain_id_override);
}
