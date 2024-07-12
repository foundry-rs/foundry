//! log/event related tests

use crate::{
    abi::SimpleStorage::{self},
    utils::{http_provider_with_signer, ws_provider_with_signer},
};
use alloy_network::EthereumWallet;
use alloy_primitives::B256;
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, Filter};
use anvil::{spawn, NodeConfig};
use futures::StreamExt;

#[tokio::test(flavor = "multi_thread")]
async fn get_past_events() {
    let (_api, handle) = spawn(NodeConfig::test()).await;

    let wallet = handle.dev_wallets().next().unwrap();
    let account = wallet.address();
    let signer: EthereumWallet = wallet.into();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let contract =
        SimpleStorage::deploy(provider.clone(), "initial value".to_string()).await.unwrap();
    let _ = contract
        .setValue("hi".to_string())
        .from(account)
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let simple_storage_address = *contract.address();

    let filter = Filter::new()
        .address(simple_storage_address)
        .topic1(B256::from(account.into_word()))
        .from_block(BlockNumberOrTag::from(0));

    let logs = provider
        .get_logs(&filter)
        .await
        .unwrap()
        .into_iter()
        .map(|log| log.log_decode::<SimpleStorage::ValueChanged>().unwrap())
        .collect::<Vec<_>>();

    // 2 events, 1 in constructor, 1 in call
    assert_eq!(logs[0].inner.newValue, "initial value");
    assert_eq!(logs[1].inner.newValue, "hi");
    assert_eq!(logs.len(), 2);

    // and we can fetch the events at a block hash
    // let hash = provider.get_block(1).await.unwrap().unwrap().hash.unwrap();
    let hash = provider
        .get_block_by_number(BlockNumberOrTag::from(1), false)
        .await
        .unwrap()
        .unwrap()
        .header
        .hash
        .unwrap();

    let filter = Filter::new()
        .address(simple_storage_address)
        .topic1(B256::from(account.into_word()))
        .at_block_hash(hash);

    let logs = provider
        .get_logs(&filter)
        .await
        .unwrap()
        .into_iter()
        .map(|log| log.log_decode::<SimpleStorage::ValueChanged>().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(logs[0].inner.newValue, "initial value");
    assert_eq!(logs.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_all_events() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let wallet = handle.dev_wallets().next().unwrap();
    let account = wallet.address();
    let signer: EthereumWallet = wallet.into();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let contract =
        SimpleStorage::deploy(provider.clone(), "initial value".to_string()).await.unwrap();

    api.anvil_set_auto_mine(false).await.unwrap();

    let pre_logs =
        provider.get_logs(&Filter::new().from_block(BlockNumberOrTag::Earliest)).await.unwrap();
    assert_eq!(pre_logs.len(), 1);

    let pre_logs =
        provider.get_logs(&Filter::new().from_block(BlockNumberOrTag::Number(0))).await.unwrap();
    assert_eq!(pre_logs.len(), 1);

    // spread logs across several blocks
    let num_tx = 10;
    let tx = contract.setValue("hi".to_string()).from(account);
    for _ in 0..num_tx {
        let tx = tx.send().await.unwrap();
        api.mine_one().await;
        tx.get_receipt().await.unwrap();
    }

    let logs =
        provider.get_logs(&Filter::new().from_block(BlockNumberOrTag::Earliest)).await.unwrap();

    let num_logs = num_tx + pre_logs.len();
    assert_eq!(logs.len(), num_logs);

    // test that logs returned from get_logs and get_transaction_receipt have
    // the same log_index, block_number, and transaction_hash
    let mut tasks = vec![];
    let mut seen_tx_hashes = std::collections::HashSet::new();
    for log in &logs {
        if seen_tx_hashes.contains(&log.transaction_hash.unwrap()) {
            continue;
        }
        tasks.push(provider.get_transaction_receipt(log.transaction_hash.unwrap()));
        seen_tx_hashes.insert(log.transaction_hash.unwrap());
    }

    let receipt_logs = futures::future::join_all(tasks)
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .flat_map(|receipt| receipt.unwrap().inner.inner.inner.receipt.logs)
        .collect::<Vec<_>>();

    assert_eq!(receipt_logs.len(), logs.len());
    for (receipt_log, log) in receipt_logs.iter().zip(logs.iter()) {
        assert_eq!(receipt_log.transaction_hash, log.transaction_hash);
        assert_eq!(receipt_log.block_number, log.block_number);
        assert_eq!(receipt_log.log_index, log.log_index);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_events() {
    let (_api, handle) = spawn(NodeConfig::test()).await;

    let wallet = handle.dev_wallets().next().unwrap();
    let account = wallet.address();
    let signer: EthereumWallet = wallet.into();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer.clone());

    let contract1 =
        SimpleStorage::deploy(provider.clone(), "initial value".to_string()).await.unwrap();

    // Spawn the event listener.
    let event1 = contract1.event_filter::<SimpleStorage::ValueChanged>();
    let mut stream1 = event1.watch().await.unwrap().into_stream();

    // Also set up a subscription for the same thing.
    let ws = ws_provider_with_signer(&handle.ws_endpoint(), signer.clone());
    let contract2 = SimpleStorage::new(*contract1.address(), ws);
    let event2 = contract2.event_filter::<SimpleStorage::ValueChanged>();
    let mut stream2 = event2.watch().await.unwrap().into_stream();

    let num_tx = 3;

    let starting_block_number = provider.get_block_number().await.unwrap();
    for i in 0..num_tx {
        contract1
            .setValue(i.to_string())
            .from(account)
            .send()
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();

        let log = stream1.next().await.unwrap().unwrap();
        let log2 = stream2.next().await.unwrap().unwrap();

        assert_eq!(log.0.newValue, log2.0.newValue);
        assert_eq!(log.0.newValue, i.to_string());
        assert_eq!(log.1.block_number.unwrap(), starting_block_number + i + 1);

        let hash = provider
            .get_block_by_number(BlockNumberOrTag::from(starting_block_number + i + 1), false)
            .await
            .unwrap()
            .unwrap()
            .header
            .hash
            .unwrap();
        assert_eq!(log.1.block_hash.unwrap(), hash);
    }
}
