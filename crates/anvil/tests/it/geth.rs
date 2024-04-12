//! tests against local geth for local debug purposes

use crate::{
    abi::VENDING_MACHINE_CONTRACT,
    utils::{http_provider, ContractInstanceCompat, DeploymentTxFactoryCompat},
};
use alloy_primitives::{Address, U256};
use alloy_rpc_types::TransactionRequest;
use foundry_compilers::{project_util::TempProject, Artifact};
use futures::StreamExt;
use std::sync::Arc;
use tokio::time::timeout;

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_geth_pending_transaction() {
    let provider = http_provider("http://127.0.0.1:8545").await;
    let accounts = provider.get_accounts().await.unwrap();
    let tx = TransactionRequest::default()
        .with_from(accounts[0].into())
        .with_to(Address::random().into())
        .with_value(U256::from(1337));

    // let client = Provider::try_from("http://127.0.0.1:8545").unwrap();
    // let accounts = client.get_accounts().await.unwrap();
    // let tx = TransactionRequest::new()
    //     .from(accounts[0])
    //     .to(Address::random())
    //     .value(1337u64)
    //     .nonce(2u64);

    // let mut watch_tx_stream =
    //     client.watch_pending_transactions().await.unwrap().transactions_unordered(1).fuse();

    // let _res = client.send_transaction(tx, None).await.unwrap();

    // let pending = timeout(std::time::Duration::from_secs(3), watch_tx_stream.next()).await;
    // pending.unwrap_err();
}
