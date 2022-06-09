//! tests against local geth for local debug purposes

use ethers::{
    abi::Address,
    prelude::{Middleware, TransactionRequest},
    providers::Provider,
};
use futures::StreamExt;
use tokio::time::timeout;

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_geth_pending_transaction() {
    let client = Provider::try_from("http://127.0.0.1:8545").unwrap();
    let accounts = client.get_accounts().await.unwrap();
    let tx = TransactionRequest::new()
        .from(accounts[0])
        .to(Address::random())
        .value(1337u64)
        .nonce(2u64);

    let mut watch_tx_stream =
        client.watch_pending_transactions().await.unwrap().transactions_unordered(1).fuse();

    let _res = client.send_transaction(tx, None).await.unwrap();

    let pending = timeout(std::time::Duration::from_secs(3), watch_tx_stream.next()).await;
    assert!(pending.is_err());
}
