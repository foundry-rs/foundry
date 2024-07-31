//! txpool related tests

use alloy_network::TransactionBuilder;
use alloy_primitives::U256;
use alloy_provider::{ext::TxPoolApi, Provider};
use alloy_rpc_types::TransactionRequest;
use alloy_serde::WithOtherFields;
use anvil::{spawn, NodeConfig};

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
    let mut txs = Vec::new();
    for _ in 0..10 {
        let tx_hash = provider.send_transaction(tx.clone()).await.unwrap();
        txs.push(tx_hash);
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
