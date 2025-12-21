//! txpool related tests

use alloy_network::{ReceiptResponse, TransactionBuilder};
use alloy_primitives::U256;
use alloy_provider::{Provider, ext::TxPoolApi};
use alloy_rpc_types::TransactionRequest;
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, spawn};

#[tokio::test(flavor = "multi_thread")]
async fn geth_txpool() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    api.anvil_set_auto_mine(false).await.unwrap();

    let account = provider.get_accounts().await.unwrap().remove(0);
    let value = U256::from(42);
    let gas_price = 221435145689u128;

    let tx = TransactionRequest::default()
        .with_to(account)
        .with_from(account)
        .with_value(value)
        .with_gas_price(gas_price);
    let tx = WithOtherFields::new(tx);

    // send a few transactions
    for _ in 0..10 {
        let _ = provider.send_transaction(tx.clone()).await.unwrap();
    }

    // we gave a 20s block time, should be plenty for us to get the txpool's content
    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.pending, 10);
    assert_eq!(status.queued, 0);

    let inspect = provider.txpool_inspect().await.unwrap();
    assert!(inspect.queued.is_empty());
    let summary = inspect.pending.get(&account).unwrap();
    for i in 0..10 {
        let tx_summary = summary.get(&i.to_string()).unwrap();
        assert_eq!(tx_summary.gas_price, gas_price);
        assert_eq!(tx_summary.value, value);
        assert_eq!(tx_summary.gas, 21000);
        assert_eq!(tx_summary.to.unwrap(), account);
    }

    let content = provider.txpool_content().await.unwrap();
    assert!(content.queued.is_empty());
    let content = content.pending.get(&account).unwrap();

    for nonce in 0..10 {
        assert!(content.contains_key(&nonce.to_string()));
    }
}

// Cf. https://github.com/foundry-rs/foundry/issues/11239
#[tokio::test(flavor = "multi_thread")]
async fn accepts_spend_after_funding_when_pool_checks_disabled() {
    // Spawn with pool balance checks disabled
    let (api, handle) = spawn(NodeConfig::test().with_disable_pool_balance_checks(true)).await;
    let provider = handle.http_provider();

    // Work with pending pool (no automine)
    api.anvil_set_auto_mine(false).await.unwrap();

    // Funder is a dev account controlled by the node
    let funder = provider.get_accounts().await.unwrap().remove(0);

    // Recipient/spender is a random address with zero balance that we'll impersonate
    let spender = alloy_primitives::Address::random();
    api.anvil_set_balance(spender, U256::from(0u64)).await.unwrap();
    api.anvil_impersonate_account(spender).await.unwrap();

    // Ensure tx1 (funding) has higher gas price so it's mined before tx2 within the same block
    let gas_price_fund = 2_000_000_000_000u128; // 2_000 gwei
    let gas_price_spend = 1_000_000_000u128; // 1 gwei

    let fund_value = U256::from(1_000_000_000_000_000_000u128); // 1 ether

    // tx1: fund spender from funder
    let tx1 = TransactionRequest::default()
        .with_from(funder)
        .with_to(spender)
        .with_value(fund_value)
        .with_gas_price(gas_price_fund);
    let tx1 = WithOtherFields::new(tx1);

    // tx2: spender attempts to send value greater than their pre-funding balance (0),
    // which would normally be rejected by pool balance checks, but should be accepted when disabled
    let spend_value = fund_value - U256::from(21_000u64) * U256::from(gas_price_spend);
    let tx2 = TransactionRequest::default()
        .with_from(spender)
        .with_to(funder)
        .with_value(spend_value)
        .with_gas_price(gas_price_spend);
    let tx2 = WithOtherFields::new(tx2);

    // Publish both transactions (funding first, then spend-before-funding-is-mined)
    let sent1 = provider.send_transaction(tx1).await.unwrap();
    let sent2 = provider.send_transaction(tx2).await.unwrap();

    // Both should be accepted into the pool (pending)
    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.pending, 2);
    assert_eq!(status.queued, 0);

    // Mine a block and ensure both succeed
    api.evm_mine(None).await.unwrap();

    let receipt1 = sent1.get_receipt().await.unwrap();
    let receipt2 = sent2.get_receipt().await.unwrap();
    assert!(receipt1.status());
    assert!(receipt2.status());
}
