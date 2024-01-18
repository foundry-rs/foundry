//! tests for subscriptions

use crate::utils::{ethers_http_provider, ethers_ws_provider};
use alloy_signer::Signer as AlloySigner;
use anvil::{spawn, NodeConfig};
use ethers::{
    contract::abigen,
    middleware::SignerMiddleware,
    prelude::{Middleware, Ws},
    providers::{JsonRpcClient, PubsubClient},
    signers::{Signer, Wallet},
    types::{Address, Block, Filter, TransactionRequest, TxHash, ValueOrArray, U256},
};
use foundry_common::types::{ToAlloy, ToEthers};
use futures::StreamExt;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_new_heads() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let provider = ethers_ws_provider(&handle.ws_endpoint());

    let blocks = provider.subscribe_blocks().await.unwrap();

    // mine a block every 1 seconds
    api.anvil_set_interval_mining(1).unwrap();

    let blocks = blocks.take(3).collect::<Vec<_>>().await;
    let block_numbers = blocks.into_iter().map(|b| b.number.unwrap().as_u64()).collect::<Vec<_>>();

    assert_eq!(block_numbers, vec![1, 2, 3]);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_logs_legacy() {
    abigen!(EmitLogs, "test-data/emit_logs.json");

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = ethers_ws_provider(&handle.ws_endpoint());

    let alloy_wallet = handle.dev_wallets().next().unwrap();
    let wallet = Wallet::new_with_signer(
        alloy_wallet.clone().signer().clone(),
        alloy_wallet.address().to_ethers(),
        alloy_wallet.chain_id().unwrap(),
    );
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let msg = "First Message".to_string();
    let contract =
        EmitLogs::deploy(Arc::clone(&client), msg.clone()).unwrap().legacy().send().await.unwrap();

    let val = contract.get_value().call().await.unwrap();
    assert_eq!(val, msg);

    // subscribe to events from the contract
    let filter = Filter::new().address(ValueOrArray::Value(contract.address()));
    let mut logs_sub = client.subscribe_logs(&filter).await.unwrap();

    // send a tx triggering an event
    let receipt = contract
        .set_value("Next Message".to_string())
        .legacy()
        .send()
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();

    // get the emitted event
    let log = logs_sub.next().await.unwrap();

    // ensure the log in the receipt is the same as received via subscription stream
    assert_eq!(receipt.logs[0], log);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_logs() {
    abigen!(EmitLogs, "test-data/emit_logs.json");

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = ethers_ws_provider(&handle.ws_endpoint());

    let alloy_wallet = handle.dev_wallets().next().unwrap();
    let wallet = Wallet::new_with_signer(
        alloy_wallet.clone().signer().clone(),
        alloy_wallet.address().to_ethers(),
        alloy_wallet.chain_id().unwrap(),
    );
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let msg = "First Message".to_string();
    let contract =
        EmitLogs::deploy(Arc::clone(&client), msg.clone()).unwrap().send().await.unwrap();

    let val = contract.get_value().call().await.unwrap();
    assert_eq!(val, msg);

    // subscribe to events from the contract
    let filter = Filter::new().address(ValueOrArray::Value(contract.address()));
    let mut logs_sub = client.subscribe_logs(&filter).await.unwrap();

    // send a tx triggering an event
    let receipt = contract
        .set_value("Next Message".to_string())
        .send()
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();

    // get the emitted event
    let log = logs_sub.next().await.unwrap();

    // ensure the log in the receipt is the same as received via subscription stream
    assert_eq!(receipt.logs[0], log);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_logs_impersonated() {
    abigen!(EmitLogs, "test-data/emit_logs.json");

    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = ethers_ws_provider(&handle.ws_endpoint());

    // impersonate account
    let impersonate = Address::random();
    let funding = U256::from(1e18 as u64);
    api.anvil_set_balance(impersonate.to_alloy(), funding.to_alloy()).await.unwrap();
    api.anvil_impersonate_account(impersonate.to_alloy()).await.unwrap();

    let alloy_wallet = handle.dev_wallets().next().unwrap();
    let wallet = Wallet::new_with_signer(
        alloy_wallet.clone().signer().clone(),
        alloy_wallet.address().to_ethers(),
        alloy_wallet.chain_id().unwrap(),
    );
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let msg = "First Message".to_string();
    let contract =
        EmitLogs::deploy(Arc::clone(&client), msg.clone()).unwrap().send().await.unwrap();

    let _val = contract.get_value().call().await.unwrap();

    // subscribe to events from the impersonated account
    let filter = Filter::new().address(ValueOrArray::Value(contract.address()));
    let mut logs_sub = client.subscribe_logs(&filter).await.unwrap();

    // send a tx triggering an event
    let data = contract.set_value("Next Message".to_string()).tx.data().cloned().unwrap();

    let tx = TransactionRequest::new().from(impersonate).to(contract.address()).data(data);

    let provider = ethers_http_provider(&handle.http_endpoint());

    let receipt = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    // get the emitted event
    let log = logs_sub.next().await.unwrap();
    // ensure the log in the receipt is the same as received via subscription stream
    assert_eq!(receipt.logs[0], log);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_filters_legacy() {
    abigen!(EmitLogs, "test-data/emit_logs.json");

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = ethers_http_provider(&handle.http_endpoint());

    let alloy_wallet = handle.dev_wallets().next().unwrap();
    let wallet = Wallet::new_with_signer(
        alloy_wallet.clone().signer().clone(),
        alloy_wallet.address().to_ethers(),
        alloy_wallet.chain_id().unwrap(),
    );
    let from = wallet.address();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let msg = "First Message".to_string();
    let contract =
        EmitLogs::deploy(Arc::clone(&client), msg.clone()).unwrap().legacy().send().await.unwrap();

    let filter = contract.value_changed_filter();
    let mut stream = filter.stream().await.unwrap();

    // send a tx triggering an event
    let _receipt = contract
        .set_value("Next Message".to_string())
        .legacy()
        .send()
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();

    // get the emitted event
    let log = stream.next().await.unwrap().unwrap();
    assert_eq!(
        log,
        ValueChangedFilter {
            author: from,
            old_value: "First Message".to_string(),
            new_value: "Next Message".to_string(),
        },
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_filters() {
    abigen!(EmitLogs, "test-data/emit_logs.json");

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = ethers_http_provider(&handle.http_endpoint());

    let alloy_wallet = handle.dev_wallets().next().unwrap();
    let wallet = Wallet::new_with_signer(
        alloy_wallet.clone().signer().clone(),
        alloy_wallet.address().to_ethers(),
        alloy_wallet.chain_id().unwrap(),
    );
    let from = wallet.address();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let msg = "First Message".to_string();
    let contract =
        EmitLogs::deploy(Arc::clone(&client), msg.clone()).unwrap().send().await.unwrap();

    let filter = contract.value_changed_filter();
    let mut stream = filter.stream().await.unwrap();

    // send a tx triggering an event
    let _receipt = contract
        .set_value("Next Message".to_string())
        .send()
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();

    // get the emitted event
    let log = stream.next().await.unwrap().unwrap();
    assert_eq!(
        log,
        ValueChangedFilter {
            author: from,
            old_value: "First Message".to_string(),
            new_value: "Next Message".to_string(),
        },
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_subscriptions() {
    let (_api, handle) =
        spawn(NodeConfig::test().with_blocktime(Some(std::time::Duration::from_secs(1)))).await;
    let ws = Ws::connect(handle.ws_endpoint()).await.unwrap();

    // Subscribing requires sending the sub request and then subscribing to
    // the returned sub_id
    let sub_id: U256 = ws.request("eth_subscribe", ["newHeads"]).await.unwrap();
    let mut stream = ws.subscribe(sub_id).unwrap();

    let mut blocks = Vec::new();
    for _ in 0..3 {
        let item = stream.next().await.unwrap();
        let block: Block<TxHash> = serde_json::from_str(item.get()).unwrap();
        blocks.push(block.number.unwrap_or_default().as_u64());
    }

    assert_eq!(blocks, vec![1, 2, 3])
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_new_heads_fast() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let provider = ethers_ws_provider(&handle.ws_endpoint());

    let blocks = provider.subscribe_blocks().await.unwrap();

    let num = 1_000u64;
    let mine_api = api.clone();
    tokio::task::spawn(async move {
        for _ in 0..num {
            mine_api.mine_one().await;
        }
    });

    // collect all the blocks
    let blocks = blocks.take(num as usize).collect::<Vec<_>>().await;
    let block_numbers = blocks.into_iter().map(|b| b.number.unwrap().as_u64()).collect::<Vec<_>>();

    let numbers = (1..=num).collect::<Vec<_>>();
    assert_eq!(block_numbers, numbers);
}
