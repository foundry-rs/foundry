//! general eth api tests

use anvil::{eth::api::CLIENT_VERSION, spawn, NodeConfig, CHAIN_ID};
use ethers::{
    prelude::Middleware,
    signers::Signer,
    types::{Block, BlockNumber, Transaction, TransactionRequest, U256},
};

#[tokio::test(flavor = "multi_thread")]
async fn can_get_block_number() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let block_num = api.block_number().unwrap();
    assert_eq!(block_num, U256::zero());

    let provider = handle.http_provider();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, block_num.as_u64().into());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_dev_get_balance() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let genesis_balance = handle.genesis_balance();
    for acc in handle.genesis_accounts() {
        let balance = provider.get_balance(acc, None).await.unwrap();
        assert_eq!(balance, genesis_balance);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_price() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let _ = provider.get_gas_price().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_accounts() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let _ = provider.get_accounts().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_client_version() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let version = provider.client_version().await.unwrap();
    assert_eq!(CLIENT_VERSION, version);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_chain_id() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let chain_id = provider.get_chainid().await.unwrap();
    assert_eq!(chain_id, CHAIN_ID.into());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_network_id() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    let chain_id = api.network_id().unwrap().unwrap();
    assert_eq!(chain_id, CHAIN_ID.to_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_block_by_number() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let amount = handle.genesis_balance().checked_div(2u64.into()).unwrap();
    // send a dummy transactions
    let tx = TransactionRequest::new().to(to).value(amount).from(from);
    let _ = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let block: Block<Transaction> = provider.get_block_with_txs(1u64).await.unwrap().unwrap();
    assert_eq!(block.transactions.len(), 1);

    let block = provider.get_block(1u64).await.unwrap().unwrap();
    assert_eq!(block.transactions.len(), 1);

    let block = provider.get_block(block.hash.unwrap()).await.unwrap().unwrap();
    assert_eq!(block.transactions.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_pending_block() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let accounts: Vec<_> = handle.dev_wallets().collect();

    let block = provider.get_block(BlockNumber::Pending).await.unwrap().unwrap();

    assert_eq!(block.number.unwrap().as_u64(), 1u64);

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num.as_u64(), 0u64);

    api.anvil_set_auto_mine(false).await.unwrap();

    let from = accounts[0].address();
    let to = accounts[1].address();
    let tx = TransactionRequest::new().to(to).value(100u64).from(from);

    let tx = provider.send_transaction(tx, None).await.unwrap();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num.as_u64(), 0u64);

    let block = provider.get_block(BlockNumber::Pending).await.unwrap().unwrap();
    assert_eq!(block.number.unwrap().as_u64(), 1u64);
    assert_eq!(block.transactions.len(), 1);
    assert_eq!(block.transactions, vec![tx.tx_hash()]);

    let block = provider.get_block_with_txs(BlockNumber::Pending).await.unwrap().unwrap();
    assert_eq!(block.number.unwrap().as_u64(), 1u64);
    assert_eq!(block.transactions.len(), 1);
}
