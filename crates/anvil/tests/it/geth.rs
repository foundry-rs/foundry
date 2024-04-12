//! tests against local geth for local debug purposes

use crate::{abi::VendingMachine, utils::http_provider};
use alloy_network::TransactionBuilder;
use alloy_primitives::{Address, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{TransactionRequest, WithOtherFields};
use futures::StreamExt;
use tokio::time::timeout;

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_geth_pending_transaction() {
    let provider = http_provider("http://127.0.0.1:8545");

    let account = provider.get_accounts().await.unwrap().remove(0);

    let tx = TransactionRequest::default()
        .with_from(account)
        .with_to(Address::random().into())
        .with_value(U256::from(1337u64))
        .with_nonce(2u64);
    let tx = WithOtherFields::new(tx);

    let mut watch_tx_stream = provider.watch_pending_transactions().await.unwrap().into_stream();

    provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let pending = timeout(std::time::Duration::from_secs(3), watch_tx_stream.next()).await;
    pending.unwrap_err();
}

// check how geth returns reverts
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_geth_revert_transaction() {
    let provider = http_provider("http://127.0.0.1:8545");

    let account = provider.get_accounts().await.unwrap().remove(0);

    // deploy successfully
    let vending_machine_builder = VendingMachine::deploy_builder(&provider).from(account);
    let vending_machine_address = vending_machine_builder.deploy().await.unwrap();
    let vending_machine = VendingMachine::new(vending_machine_address, &provider);

    // expect revert
    let call = vending_machine.buyRevert(U256::from(10u64)).from(account).call().await;
    let err = call.unwrap_err().to_string();
    assert!(err.contains("execution reverted: Not enough Ether provided."));
    assert!(err.contains("code: 3"));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_geth_low_gas_limit() {
    let provider = http_provider("http://127.0.0.1:8545");

    let account = provider.get_accounts().await.unwrap().remove(0);

    let gas_limit = 21_000u128 - 1;
    let tx = TransactionRequest::default()
        .from(account)
        .to(Address::random().into())
        .value(U256::from(1337u64))
        .gas_limit(gas_limit);
    let tx = WithOtherFields::new(tx);

    let resp = provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await;
    let err = resp.unwrap_err().to_string();
    assert!(err.contains("intrinsic gas too low"));
}
