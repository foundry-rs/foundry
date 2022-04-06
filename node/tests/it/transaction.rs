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

    let tx = TransactionRequest::new()
        .to(to)
        .value(amount)
        .gas_price(handle.gas_price())
        .gas(U256::max_value())
        .from(from);

    let _balance_before = provider.get_balance(from, None).await.unwrap();

    // broadcast it via the eth_sendTransaction API
    let tx = provider.send_transaction(tx, None).await.unwrap().await;

    dbg!(tx);
    // provider.get_transaction();
}
