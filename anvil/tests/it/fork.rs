//! various fork related test

use crate::{abi::*, utils};
use anvil::{eth::EthApi, spawn, NodeConfig, NodeHandle};
use anvil_core::types::Forking;
use ethers::{
    core::rand,
    prelude::{Bytes, LocalWallet, Middleware, SignerMiddleware},
    signers::Signer,
    types::{
        transaction::eip2718::TypedTransaction, Address, BlockNumber, Chain, TransactionRequest,
        U256,
    },
};
use foundry_utils::rpc;
use std::{sync::Arc, time::Duration};

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
    let _history = api.fee_history(count.into(), BlockNumber::Latest, vec![]).await.unwrap();
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
    let fork_db = fork.database.read().await;
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
        NodeConfig::test().with_eth_rpc_url(Some(rpc::next_rinkeby_http_rpc_endpoint())).silent(),
    )
    .await;
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

#[tokio::test(flavor = "multi_thread")]
async fn can_reset_properly() {
    let (origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let account = origin_handle.dev_accounts().next().unwrap();
    let origin_provider = origin_handle.http_provider();
    let origin_nonce = 1u64.into();
    origin_api.anvil_set_nonce(account, origin_nonce).await.unwrap();

    assert_eq!(origin_nonce, origin_provider.get_transaction_count(account, None).await.unwrap());

    let (fork_api, fork_handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(origin_handle.http_endpoint()))).await;

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

    let start = std::time::Instant::now();

    let block = provider.get_block(BLOCK_NUMBER).await.unwrap().unwrap();
    assert_eq!(block.timestamp.as_u64(), BLOCK_TIMESTAMP);

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();

    let tx = TransactionRequest::new().to(Address::random()).value(1337u64).from(from);
    let _tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();

    // ensure the diff between the new mined block and the original block is within the elapsed time
    let elapsed = start.elapsed().as_secs() + 1;
    let diff = block.timestamp - BLOCK_TIMESTAMP;
    assert!(diff <= elapsed.into());

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

    api.anvil_set_balance(wallet.address(), U256::from(1e18 as u64)).await.unwrap();

    let provider = SignerMiddleware::new(handle.http_provider(), wallet);

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
    api.anvil_set_balance(wallet.address(), U256::from(1000e18 as u64)).await.unwrap();

    let provider = Arc::new(SignerMiddleware::new(handle.http_provider(), wallet.clone()));

    // pick a random nft <https://opensea.io/assets/ethereum/0x9c8ff314c9bc7f6e59a9d9225fb22946427edc03/154>
    let nouns_addr: Address = "0x9c8ff314c9bc7f6e59a9d9225fb22946427edc03".parse().unwrap();

    let owner: Address = "0x052564eb0fd8b340803df55def89c25c432f43f4".parse().unwrap();
    let token_id: U256 = 154u64.into();

    let nouns = Erc721::new(nouns_addr, Arc::clone(&provider));

    let real_onwer = nouns.owner_of(token_id).call().await.unwrap();
    assert_eq!(real_onwer, owner);
    let approval = nouns.set_approval_for_all(nouns_addr, true);
    let tx = approval.send().await.unwrap().await.unwrap().unwrap();
    assert_eq!(tx.status, Some(1u64.into()));

    // transfer: impersonate real owner and transfer nft
    api.anvil_impersonate_account(real_onwer).await.unwrap();

    api.anvil_set_balance(real_onwer, U256::from(10000e18 as u64)).await.unwrap();

    let call = nouns.transfer_from(real_onwer, wallet.address(), token_id);
    let mut tx: TypedTransaction = call.tx;
    tx.set_from(real_onwer);
    provider.fill_transaction(&mut tx, None).await.unwrap();
    let tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();
    assert_eq!(tx.status, Some(1u64.into()));

    let real_onwer = nouns.owner_of(token_id).call().await.unwrap();
    assert_eq!(real_onwer, wallet.address());
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
    assert_eq!(eth_chain_id.unwrap().unwrap().as_u64(), 3145u64);
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
    api.anvil_impersonate_account(sender).await.unwrap();

    let provider = handle.http_provider();

    let input: Bytes = "0xfb0f3ee1000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000003ff2e795f5000000000000000000000000000023f28ae3e9756ba982a6290f9081b6a84900b758000000000000000000000000004c00500000ad104d7dbd00e3ae0a5c00560c0000000000000000000000000003235b597a78eabcb08ffcb4d97411073211dbcb0000000000000000000000000000000000000000000000000000000000000e72000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000062ad47c20000000000000000000000000000000000000000000000000000000062d43104000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000df44e65d2a2cf40000007b02230091a7ed01230072f7006a004d60a8d4e71d599b8104250f00000000007b02230091a7ed01230072f7006a004d60a8d4e71d599b8104250f00000000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000024000000000000000000000000000000000000000000000000000000000000002e000000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000001c6bf526340000000000000000000000000008de9c5a032463c561423387a9648c5c7bcc5bc900000000000000000000000000000000000000000000000000005543df729c0000000000000000000000000006eb234847a9e3a546539aac57a071c01dc3f398600000000000000000000000000000000000000000000000000000000000000416d39b5352353a22cf2d44faa696c2089b03137a13b5acfee0366306f2678fede043bc8c7e422f6f13a3453295a4a063dac7ee6216ab7bade299690afc77397a51c00000000000000000000000000000000000000000000000000000000000000".parse().unwrap();
    let to: Address = "0x00000000006c3852cbef3e08e8df289169ede581".parse().unwrap();
    let tx = TransactionRequest::new()
        .from(sender)
        .to(to)
        .value(20000000000000000u64)
        .data(input)
        .gas_price(22180711707u64);

    let tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();
    assert_eq!(tx.status, Some(1u64.into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_base_fee() {
    let (api, handle) = spawn(fork_config()).await;

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();

    let provider = handle.http_provider();

    api.anvil_set_next_block_base_fee_per_gas(U256::zero()).await.unwrap();

    let addr = Address::random();
    let val = 1337u64;
    let tx = TransactionRequest::new().from(from).to(addr).value(val).gas(0u64);

    let _res = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_init_base_fee() {
    let (api, handle) = spawn(fork_config().with_fork_block_number(Some(13184859u64))).await;

    let provider = handle.http_provider();

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
