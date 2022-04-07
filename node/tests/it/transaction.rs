use crate::{init_tracing, next_port};
use ethers::prelude::{abigen, Middleware, Signer, SignerMiddleware, TransactionRequest};
use foundry_node::{spawn, NodeConfig};
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
async fn can_send_transaction() {
    init_tracing();

    let (_api, _handle) = spawn(NodeConfig::default().port(next_port()));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_transfer_eth() {
    init_tracing();
    let (_api, handle) = spawn(NodeConfig::default().port(next_port()));
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
async fn can_deploy_greeter() {
    abigen!(Greeter, "test-data/greeter.json");

    init_tracing();
    let (_api, handle) = spawn(NodeConfig::default().port(next_port()));
    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let greeter_contract =
        Greeter::deploy(client, "Hello World!".to_string()).unwrap().legacy().send().await.unwrap();

    dbg!(greeter_contract);
}
