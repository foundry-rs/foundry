//! txpool related tests

use anvil::{spawn, NodeConfig};
use ethers::{
    prelude::Middleware,
    types::{TransactionRequest, U256},
};

#[tokio::test(flavor = "multi_thread")]
async fn geth_txpool() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    api.anvil_set_auto_mine(false).await.unwrap();

    let account = provider.get_accounts().await.unwrap()[0];
    let value: u64 = 42;
    let gas_price: U256 = 221435145689u64.into();
    let tx = TransactionRequest::new().to(account).from(account).value(value).gas_price(gas_price);

    // send a few transactions
    let mut txs = Vec::new();
    for _ in 0..10 {
        let tx_hash = provider.send_transaction(tx.clone(), None).await.unwrap();
        txs.push(tx_hash);
    }

    // we gave a 20s block time, should be plenty for us to get the txpool's content
    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.pending.as_u64(), 10);
    assert_eq!(status.queued.as_u64(), 0);

    let inspect = provider.txpool_inspect().await.unwrap();
    assert!(inspect.queued.is_empty());
    let summary = inspect.pending.get(&account).unwrap();
    for i in 0..10 {
        let tx_summary = summary.get(&i.to_string()).unwrap();
        assert_eq!(tx_summary.gas_price, gas_price);
        assert_eq!(tx_summary.value, value.into());
        assert_eq!(tx_summary.gas, 21000.into());
        assert_eq!(tx_summary.to.unwrap(), account);
    }

    let content = provider.txpool_content().await.unwrap();
    assert!(content.queued.is_empty());
    let content = content.pending.get(&account).unwrap();

    for nonce in 0..10 {
        assert!(content.contains_key(&nonce.to_string()));
    }
}
