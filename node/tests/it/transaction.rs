use crate::{init_tracing, next_port};
use ethers::{
    providers::Middleware,
    signers::Signer,
    types::{TransactionRequest, U256},
};
use foundry_node::{spawn, NodeConfig};

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

    let amount = handle.genesis_balance().checked_div(2u64.into()).unwrap();

    let tx =
        TransactionRequest::new().to(to).value(amount).gas_price(handle.gas_price()).from(from);

    // craft the tx
    let tx = TransactionRequest::new().to(to).value(1000).from(from); // specify the `from` field so that the client knows which account to use

    let balance_before = provider.get_balance(from, None).await.unwrap();

    // broadcast it via the eth_sendTransaction API
    let tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap();

    println!("{}", serde_json::to_string(&tx).unwrap());

    let nonce1 =
        provider.get_transaction_count(from, Some(BlockNumber::Latest.into())).await.unwrap();

    assert!(nonce2 < nonce1);

    let balance_after = provider.get_balance(from, None).await.unwrap();
    assert!(balance_after < balance_before);

    dbg!(tx);
    // provider.get_transaction();
}
