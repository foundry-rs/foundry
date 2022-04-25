use crate::{init_tracing, next_port};
use anvil::{spawn, NodeConfig};
use ethers::prelude::{abigen, Middleware, Signer, SignerMiddleware, TransactionRequest};
use futures::StreamExt;
use std::{sync::Arc, time::Duration};
use tokio::time::timeout;

#[tokio::test(flavor = "multi_thread")]
async fn can_transfer_eth() {
    let (_api, handle) = spawn(NodeConfig::test().port(next_port())).await;
    let provider = handle.http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let nonce = provider.get_transaction_count(from, None).await.unwrap();
    assert!(nonce.is_zero());

    let balance_before = provider.get_balance(to, None).await.unwrap();

    let amount = handle.genesis_balance().checked_div(2u64.into()).unwrap();

    // craft the tx
    // specify the `from` field so that the client knows which account to use
    let tx = TransactionRequest::new().to(to).value(amount).from(from);

    // broadcast it via the eth_sendTransaction API
    let tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    assert_eq!(tx.block_number, Some(1u64.into()));
    assert_eq!(tx.transaction_index, 0u64.into());

    let nonce = provider.get_transaction_count(from, None).await.unwrap();

    assert_eq!(nonce, 1u64.into());

    let to_balance = provider.get_balance(to, None).await.unwrap();

    assert_eq!(balance_before.saturating_add(amount), to_balance);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_respect_nonces() {
    init_tracing();
    let (api, handle) = spawn(NodeConfig::test().port(next_port())).await;
    let provider = handle.http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let nonce = provider.get_transaction_count(from, None).await.unwrap();
    let amount = handle.genesis_balance().checked_div(3u64.into()).unwrap();

    // set higher nonce
    let tx = TransactionRequest::new().to(to).value(amount).from(from);

    // send the transaction with higher nonce than on chain
    let higher_pending_tx =
        provider.send_transaction(tx.clone().nonce(nonce + 1u64), None).await.unwrap();

    // ensure the listener for ready transactions times out
    let mut listener = api.new_ready_transactions();
    let res = timeout(Duration::from_millis(1500), async move { listener.next().await }).await;
    assert!(res.is_err());

    // send with the actual nonce which is mined immediately
    let tx =
        provider.send_transaction(tx.nonce(nonce), None).await.unwrap().await.unwrap().unwrap();

    // this will unblock the currently pending tx
    let higher_tx = higher_pending_tx.await.unwrap().unwrap();

    let block = provider.get_block(1u64).await.unwrap().unwrap();
    assert_eq!(2, block.transactions.len());
    assert_eq!(vec![tx.transaction_hash, higher_tx.transaction_hash], block.transactions);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_deploy_greeter() {
    abigen!(Greeter, "test-data/greeter.json");

    let (_api, handle) = spawn(NodeConfig::test().port(next_port())).await;
    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let greeter_contract =
        Greeter::deploy(client, "Hello World!".to_string()).unwrap().legacy().send().await.unwrap();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_deploy_greeter_ws() {
    abigen!(Greeter, "test-data/greeter.json");

    let (_api, handle) = spawn(NodeConfig::test().port(next_port())).await;
    let provider = handle.ws_provider().await;

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let greeter_contract =
        Greeter::deploy(client, "Hello World!".to_string()).unwrap().legacy().send().await.unwrap();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);
}
