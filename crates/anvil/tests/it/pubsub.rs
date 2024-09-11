//! tests for subscriptions

use crate::utils::{connect_pubsub, connect_pubsub_with_wallet};
use alloy_network::{EthereumWallet, TransactionBuilder};
use alloy_primitives::{Address, U256};
use alloy_provider::Provider;
use alloy_pubsub::Subscription;
use alloy_rpc_types::{Block as AlloyBlock, Filter, TransactionRequest};
use alloy_serde::WithOtherFields;
use alloy_sol_types::sol;
use anvil::{spawn, NodeConfig};
use futures::StreamExt;

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_new_heads() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let provider = connect_pubsub(&handle.ws_endpoint()).await;

    let blocks = provider.subscribe_blocks().await.unwrap();

    // mine a block every 1 seconds
    api.anvil_set_interval_mining(1).unwrap();

    let blocks = blocks.into_stream().take(3).collect::<Vec<_>>().await;
    let block_numbers = blocks.into_iter().map(|b| b.header.number).collect::<Vec<_>>();

    assert_eq!(block_numbers, vec![1, 2, 3]);
}

sol!(
    #[sol(rpc)]
    EmitLogs,
    "test-data/emit_logs.json"
);
// FIXME: Use .legacy() in tx when implemented in alloy
#[tokio::test(flavor = "multi_thread")]
async fn test_sub_logs_legacy() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let provider = connect_pubsub(&handle.ws_endpoint()).await;

    let msg = "First Message".to_string();
    let contract_addr = EmitLogs::deploy_builder(provider.clone(), msg.clone())
        .from(wallet.address())
        .deploy()
        .await
        .unwrap();
    let contract = EmitLogs::new(contract_addr, provider.clone());

    let val = contract.getValue().call().await.unwrap();
    assert_eq!(val._0, msg);

    // subscribe to events from the contract
    let filter = Filter::new().address(contract.address().to_owned());
    let logs_sub = provider.subscribe_logs(&filter).await.unwrap();

    // send a tx triggering an event
    // FIXME: Use .legacy() in tx
    let receipt = contract
        .setValue("Next Message".to_string())
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let mut logs_sub = logs_sub.into_stream();
    // get the emitted event
    let log = logs_sub.next().await.unwrap();

    // ensure the log in the receipt is the same as received via subscription stream
    assert_eq!(receipt.inner.logs()[0], log);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_logs() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let provider = connect_pubsub(&handle.ws_endpoint()).await;

    let msg = "First Message".to_string();
    let contract_addr = EmitLogs::deploy_builder(provider.clone(), msg.clone())
        .from(wallet.address())
        .deploy()
        .await
        .unwrap();
    let contract = EmitLogs::new(contract_addr, provider.clone());

    let val = contract.getValue().call().await.unwrap();
    assert_eq!(val._0, msg);

    // subscribe to events from the contract
    let filter = Filter::new().address(contract.address().to_owned());
    let logs_sub = provider.subscribe_logs(&filter).await.unwrap();

    // send a tx triggering an event
    let receipt = contract
        .setValue("Next Message".to_string())
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let mut logs_sub = logs_sub.into_stream();
    // get the emitted event
    let log = logs_sub.next().await.unwrap();

    // ensure the log in the receipt is the same as received via subscription stream
    assert_eq!(receipt.inner.logs()[0], log);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_logs_impersonated() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let provider =
        connect_pubsub_with_wallet(&handle.ws_endpoint(), EthereumWallet::from(wallet.clone()))
            .await;

    // impersonate account
    let impersonate = Address::random();
    let funding = U256::from(1e18 as u64);
    api.anvil_set_balance(impersonate, funding).await.unwrap();
    api.anvil_impersonate_account(impersonate).await.unwrap();

    let msg = "First Message".to_string();
    let contract = EmitLogs::deploy(provider.clone(), msg.clone()).await.unwrap();

    let _val = contract.getValue().call().await.unwrap();

    // subscribe to events from the impersonated account
    let filter = Filter::new().address(contract.address().to_owned());
    let logs_sub = provider.subscribe_logs(&filter).await.unwrap();

    // send a tx triggering an event
    let data = contract.setValue("Next Message".to_string());
    let data = data.calldata().clone();

    let tx =
        TransactionRequest::default().from(impersonate).to(*contract.address()).with_input(data);

    let tx = WithOtherFields::new(tx);
    let provider = handle.http_provider();

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let mut logs_sub = logs_sub.into_stream();
    // get the emitted event
    let log = logs_sub.next().await.unwrap();
    // ensure the log in the receipt is the same as received via subscription stream
    assert_eq!(receipt.inner.inner.logs()[0], log);
}

// FIXME: Use legacy() in tx when implemented in alloy
#[tokio::test(flavor = "multi_thread")]
async fn test_filters_legacy() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let provider =
        connect_pubsub_with_wallet(&handle.ws_endpoint(), EthereumWallet::from(wallet.clone()))
            .await;

    let from = wallet.address();

    let msg = "First Message".to_string();

    // FIXME: Use legacy() in tx when implemented in alloy
    let contract = EmitLogs::deploy(provider.clone(), msg.clone()).await.unwrap();

    let stream = contract.ValueChanged_filter().subscribe().await.unwrap();

    // send a tx triggering an event
    // FIXME: Use legacy() in tx when implemented in alloy
    let _receipt = contract
        .setValue("Next Message".to_string())
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let mut log = stream.into_stream();
    // get the emitted event
    let (value_changed, _log) = log.next().await.unwrap().unwrap();

    assert_eq!(value_changed.author, from);
    assert_eq!(value_changed.oldValue, "First Message".to_string());
    assert_eq!(value_changed.newValue, "Next Message".to_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_filters() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let provider =
        connect_pubsub_with_wallet(&handle.ws_endpoint(), EthereumWallet::from(wallet.clone()))
            .await;

    let from = wallet.address();

    let msg = "First Message".to_string();

    let contract = EmitLogs::deploy(provider.clone(), msg.clone()).await.unwrap();

    let stream = contract.ValueChanged_filter().subscribe().await.unwrap();

    // send a tx triggering an event
    let _receipt = contract
        .setValue("Next Message".to_string())
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let mut log = stream.into_stream();
    // get the emitted event
    let (value_changed, _log) = log.next().await.unwrap().unwrap();

    assert_eq!(value_changed.author, from);
    assert_eq!(value_changed.oldValue, "First Message".to_string());
    assert_eq!(value_changed.newValue, "Next Message".to_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_subscriptions() {
    let (_api, handle) =
        spawn(NodeConfig::test().with_blocktime(Some(std::time::Duration::from_secs(1)))).await;
    let provider = connect_pubsub(&handle.ws_endpoint()).await;
    let sub_id = provider.raw_request("eth_subscribe".into(), ["newHeads"]).await.unwrap();
    let stream: Subscription<AlloyBlock> = provider.get_subscription(sub_id).await.unwrap();
    let blocks = stream
        .into_stream()
        .take(3)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .map(|b| b.header.number)
        .collect::<Vec<_>>();

    assert_eq!(blocks, vec![1, 2, 3])
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_new_heads_fast() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let provider = connect_pubsub(&handle.ws_endpoint()).await;

    let blocks = provider.subscribe_blocks().await.unwrap();
    let mut blocks = blocks.into_stream();

    let num = 1000u64;

    let mut block_numbers = Vec::new();
    for _ in 0..num {
        api.mine_one().await;
        let block_number = blocks.next().await.unwrap().header.number;
        block_numbers.push(block_number);
    }

    println!("Collected {} blocks", block_numbers.len());

    let numbers = (1..=num).collect::<Vec<_>>();
    assert_eq!(block_numbers, numbers);
}
