use crate::next_port;
use anvil::{spawn, NodeConfig};
use ethers::prelude::{Middleware, Signer, TransactionRequest};

#[tokio::test(flavor = "multi_thread")]
async fn test_get_transfer_parity_traces() {
    let (_api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let amount = handle.genesis_balance().checked_div(2u64.into()).unwrap();
    // specify the `from` field so that the client knows which account to use
    let tx = TransactionRequest::new().to(to).value(amount).from(from);

    // broadcast it via the eth_sendTransaction API
    let tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let traces = provider.trace_transaction(tx.transaction_hash).await.unwrap();
    assert!(!traces.is_empty());

    let num = provider.get_block_number().await.unwrap();
    let block_traces = provider.trace_block(num.into()).await.unwrap();
    assert!(!block_traces.is_empty());

    assert_eq!(traces, block_traces);
}
