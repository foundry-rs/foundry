//! tests for subscriptions

use crate::utils::{connect_pubsub, connect_pubsub_with_wallet};
use alloy_network::{EthereumWallet, ReceiptResponse, TransactionBuilder};
use alloy_primitives::{Address, B256, U256};
use alloy_provider::Provider;
use alloy_pubsub::Subscription;
use alloy_rpc_types::{
    Block as AlloyBlock, Filter, TransactionRequest, pubsub::TransactionReceiptsParams,
};
use alloy_serde::WithOtherFields;
use alloy_sol_types::sol;
use anvil::{NodeConfig, spawn};
use anvil_core::types::{ReorgOptions, TransactionData};
use foundry_primitives::FoundryTxReceipt;
use futures::StreamExt;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_new_heads() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let provider = connect_pubsub(&handle.ws_endpoint()).await;

    let blocks = provider.subscribe_blocks().await.unwrap();

    // mine a block every 1 seconds
    api.anvil_set_interval_mining(1).unwrap();

    let blocks = blocks.into_stream().take(3).collect::<Vec<_>>().await;
    let block_numbers = blocks.into_iter().map(|b| b.number).collect::<Vec<_>>();

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
    assert_eq!(val, msg);

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
    assert_eq!(val, msg);

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

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_logs_reorg_removed() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let provider =
        connect_pubsub_with_wallet(&handle.ws_endpoint(), EthereumWallet::from(wallet.clone()))
            .await;

    let contract = EmitLogs::deploy(provider.clone(), "initial".to_string()).await.unwrap();

    // subscribe to events from the contract
    let filter = Filter::new().address(*contract.address());
    let logs_sub = provider.subscribe_logs(&filter).await.unwrap();
    let mut logs_sub = logs_sub.into_stream();

    // emit events in two consecutive blocks
    contract.setValue("first".to_string()).send().await.unwrap().get_receipt().await.unwrap();
    contract.setValue("second".to_string()).send().await.unwrap().get_receipt().await.unwrap();

    let first_log = logs_sub.next().await.unwrap();
    let second_log = logs_sub.next().await.unwrap();
    assert!(!first_log.removed);
    assert!(!second_log.removed);

    // reorg out both blocks and replace the first one with a block that emits another event
    let data = contract.setValue("reorged".to_string()).calldata().clone();
    let tx = TransactionRequest::default()
        .from(wallet.address())
        .to(*contract.address())
        .with_input(data);
    api.anvil_reorg(ReorgOptions {
        depth: 2,
        tx_block_pairs: vec![(TransactionData::JSON(tx.into()), 0)],
    })
    .await
    .unwrap();

    // the logs of the reorged out blocks are delivered again, marked as removed
    let mut expected = first_log.clone();
    expected.removed = true;
    assert_eq!(logs_sub.next().await.unwrap(), expected);

    let mut expected = second_log.clone();
    expected.removed = true;
    assert_eq!(logs_sub.next().await.unwrap(), expected);

    // followed by the logs of the new chain
    let new_log = logs_sub.next().await.unwrap();
    assert!(!new_log.removed);
    assert_eq!(new_log.block_number, first_log.block_number);
    assert_ne!(new_log.block_hash, first_log.block_hash);
    let value_changed = new_log.log_decode::<EmitLogs::ValueChanged>().unwrap();
    assert_eq!(value_changed.inner.newValue, "reorged");

    // eth_getLogs only returns the logs of the new canonical chain
    let canonical_logs = provider
        .get_logs(&Filter::new().address(*contract.address()).from_block(0u64))
        .await
        .unwrap();
    assert!(canonical_logs.iter().all(|log| !log.removed));
    let tx_hashes =
        canonical_logs.iter().map(|log| log.transaction_hash.unwrap()).collect::<Vec<_>>();
    assert!(!tx_hashes.contains(&first_log.transaction_hash.unwrap()));
    assert!(!tx_hashes.contains(&second_log.transaction_hash.unwrap()));
    assert!(tx_hashes.contains(&new_log.transaction_hash.unwrap()));
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
async fn test_sub_syncing_delivers_initial_status() {
    let (_api, handle) = spawn(NodeConfig::test()).await;

    let provider = connect_pubsub(&handle.ws_endpoint()).await;
    let sub_id = provider.raw_request("eth_subscribe".into(), ["syncing"]).await.unwrap();
    let stream: Subscription<serde_json::Value> = provider.get_subscription(sub_id).await.unwrap();
    let mut stream = stream.into_stream();

    let initial = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("timed out waiting for initial sync status")
        .expect("subscription ended unexpectedly");

    assert_eq!(initial, serde_json::json!(false));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_transaction_receipts() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let ws_provider = connect_pubsub(&handle.ws_endpoint()).await;
    let http_provider = handle.http_provider();

    api.anvil_set_auto_mine(false).await.unwrap();

    let accounts = http_provider.get_accounts().await.unwrap();
    let from = accounts[0];
    let to = accounts[1];

    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_value(U256::ONE)
        .with_nonce(0);
    let first = http_provider.send_transaction(WithOtherFields::new(tx)).await.unwrap();

    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_value(U256::from(2))
        .with_nonce(1);
    let second = http_provider.send_transaction(WithOtherFields::new(tx)).await.unwrap();

    let all_id =
        ws_provider.raw_request("eth_subscribe".into(), ["transactionReceipts"]).await.unwrap();
    let all_stream: Subscription<Vec<FoundryTxReceipt>> =
        ws_provider.get_subscription(all_id).await.unwrap();
    let mut all_stream = all_stream.into_stream();

    let filter = TransactionReceiptsParams { transaction_hashes: Some(vec![*second.tx_hash()]) };
    let filtered_id = ws_provider
        .raw_request("eth_subscribe".into(), ("transactionReceipts", filter))
        .await
        .unwrap();
    let filtered_stream: Subscription<Vec<FoundryTxReceipt>> =
        ws_provider.get_subscription(filtered_id).await.unwrap();
    let mut filtered_stream = filtered_stream.into_stream();

    api.mine_one().await.unwrap();

    let all_receipts = tokio::time::timeout(Duration::from_secs(5), all_stream.next())
        .await
        .expect("timed out waiting for transaction receipts")
        .expect("subscription ended unexpectedly");
    let filtered_receipts = tokio::time::timeout(Duration::from_secs(5), filtered_stream.next())
        .await
        .expect("timed out waiting for filtered transaction receipts")
        .expect("subscription ended unexpectedly");

    assert_eq!(all_receipts.len(), 2);
    assert_eq!(all_receipts[0].transaction_hash(), *first.tx_hash());
    assert_eq!(all_receipts[1].transaction_hash(), *second.tx_hash());

    assert_eq!(filtered_receipts.len(), 1);
    assert_eq!(filtered_receipts[0].transaction_hash(), *second.tx_hash());
}

// A receipt subscription must drop its block listener once the subscriber unsubscribes, even if its
// filter never matched any transaction.
#[tokio::test(flavor = "multi_thread")]
async fn test_sub_transaction_receipts_cleanup_on_unsubscribe() {
    let (api, _handle) = spawn(NodeConfig::test()).await;
    api.anvil_set_auto_mine(false).await.unwrap();

    let baseline = api.backend.new_block_listeners_count();

    // Filter on a hash that never gets mined, so the task can only exit via the dropped receiver.
    let filter = TransactionReceiptsParams { transaction_hashes: Some(vec![B256::random()]) };
    let rx = api.transaction_receipts_subscription(filter);
    assert_eq!(api.backend.new_block_listeners_count(), baseline + 1);

    drop(rx);

    // Let the task observe the closed channel, then mine to trigger listener pruning.
    tokio::time::sleep(Duration::from_millis(200)).await;
    api.mine_one().await.unwrap();
    api.mine_one().await.unwrap();

    assert_eq!(
        api.backend.new_block_listeners_count(),
        baseline,
        "subscription task should terminate after unsubscribe"
    );
}

#[expect(clippy::disallowed_macros)]
#[tokio::test(flavor = "multi_thread")]
async fn test_sub_new_heads_fast() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let provider = connect_pubsub(&handle.ws_endpoint()).await;

    let blocks = provider.subscribe_blocks().await.unwrap();
    let mut blocks = blocks.into_stream();

    let num = 1000u64;

    let mut block_numbers = Vec::new();
    for _ in 0..num {
        api.mine_one().await.unwrap();
        let block_number = blocks.next().await.unwrap().number;
        block_numbers.push(block_number);
    }

    println!("Collected {} blocks", block_numbers.len());

    let numbers = (1..=num).collect::<Vec<_>>();
    assert_eq!(block_numbers, numbers);
}

mod backpressure {
    use super::*;
    use alloy_provider::RootProvider;
    use alloy_rpc_types::{FilteredParams, TransactionReceipt};
    use anvil::{
        NodeHandle,
        eth::EthApi,
        pubsub::{EthSubscription, LogsSubscription},
    };
    use anvil_core::eth::subscription::SubscriptionId;
    use foundry_primitives::FoundryNetwork;
    use std::task::{Context, Poll, Waker};

    struct LogsFixture {
        api: EthApi<FoundryNetwork>,
        _handle: NodeHandle,
        sender: Address,
        contract: EmitLogs::EmitLogsInstance<RootProvider>,
    }

    impl LogsFixture {
        async fn new() -> Self {
            let (api, handle) =
                spawn(NodeConfig::test().with_transaction_block_keeper(Some(1usize))).await;
            let provider = connect_pubsub(&handle.ws_endpoint()).await;
            let sender = handle.dev_wallets().next().unwrap().address();
            let address = EmitLogs::deploy_builder(provider.clone(), "initial".to_owned())
                .from(sender)
                .deploy()
                .await
                .unwrap();
            Self { api, _handle: handle, sender, contract: EmitLogs::new(address, provider) }
        }

        fn logs(&self, filter: Filter) -> EthSubscription<FoundryNetwork> {
            EthSubscription::Logs(Box::new(LogsSubscription {
                blocks: self.api.new_block_notifications(),
                storage: self.api.storage_info(),
                filter: FilteredParams::new(Some(filter)),
                queued: Default::default(),
                id: SubscriptionId::Number(1),
            }))
        }

        async fn emit(&self, message: &str) -> TransactionReceipt {
            let receipt = self
                .contract
                .setValue(message.to_owned())
                .from(self.sender)
                .send()
                .await
                .unwrap()
                .get_receipt()
                .await
                .unwrap();
            assert_eq!(receipt.inner.logs().len(), 1);
            receipt
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lagged_logs_subscription_skips_filter_excluded_pruned_blocks() {
        let fixture = LogsFixture::new().await;
        let api = &fixture.api;
        let current_block = api.backend.best_number();
        let matching_block = current_block + 3;
        let mut subscriptions = [
            ("fromBlock", fixture.logs(Filter::new().from_block(matching_block))),
            ("toBlock", fixture.logs(Filter::new().to_block(current_block))),
            ("blockHash", fixture.logs(Filter::new().at_block_hash(B256::ZERO))),
        ];
        let mut mined_blocks = api.new_block_notifications();

        let receipt = fixture.emit("excluded notification").await;
        let excluded_hash = receipt.block_hash.unwrap();
        api.mine_one().await.unwrap();
        let latest_hash = api.backend.best_hash();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let notification = mined_blocks.next().await.unwrap();
                if notification.as_new_block().is_some_and(|block| block.hash == latest_hash) {
                    break;
                }
            }
        })
        .await
        .expect("mined block notification was not published");
        assert_eq!(api.backend.mined_receipts(excluded_hash), Some(vec![]));

        let mut cx = Context::from_waker(Waker::noop());
        for (filter, subscription) in &mut subscriptions {
            assert_eq!(
                subscription.poll_next_unpin(&mut cx),
                Poll::Pending,
                "{filter} must skip excluded pruned blocks without ending the subscription"
            );
        }

        let receipt = fixture.emit("matching notification").await;
        assert_eq!(receipt.block_number, Some(matching_block));
        let notification = tokio::time::timeout(Duration::from_secs(5), subscriptions[0].1.next())
            .await
            .expect("matching notification was not delivered")
            .expect("subscription ended before its matching block");
        assert_eq!(
            notification,
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "eth_subscription",
                "params": {"subscription": 1, "result": receipt.inner.logs()[0]},
            })
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lagged_logs_subscription_ends_after_receipt_pruning() {
        let fixture = LogsFixture::new().await;
        let api = &fixture.api;
        let mut logs = fixture.logs(Filter::new().address(*fixture.contract.address()));

        let receipt = fixture.emit("lost notification").await;
        let block_hash = receipt.block_hash.unwrap();
        api.mine_one().await.unwrap();
        assert_eq!(api.backend.mined_receipts(block_hash), Some(vec![]));
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), logs.next())
                .await
                .expect("lagged logs subscription did not end after receipt pruning"),
            None
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lagged_logs_subscription_skips_reverted_blocks() {
        let fixture = LogsFixture::new().await;
        let api = &fixture.api;
        let mut logs = fixture.logs(Filter::new().address(*fixture.contract.address()));
        let snapshot = api.evm_snapshot().await.unwrap();

        let receipt = fixture.emit("reverted notification").await;
        assert!(api.evm_revert(snapshot).await.unwrap());
        assert!(api.storage_info().block(receipt.block_hash.unwrap()).is_none());

        let mut log = receipt.inner.logs()[0].clone();
        log.removed = true;
        let notification = tokio::time::timeout(Duration::from_secs(5), logs.next())
            .await
            .expect("removed log was not delivered")
            .expect("subscription ended at the reverted block");
        assert_eq!(
            notification,
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "eth_subscription",
                "params": {"subscription": 1, "result": log},
            })
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lagged_logs_subscription_skips_pruned_zero_bloom_blocks() {
        let fixture = LogsFixture::new().await;
        let api = &fixture.api;
        // Finish publishing setup notifications before installing the subscription.
        api.mine_one().await.unwrap();
        let mut logs = fixture.logs(Filter::new());
        let recipient = fixture._handle.dev_wallets().nth(1).unwrap().address();
        let receipt = fixture
            .contract
            .provider()
            .send_transaction(
                TransactionRequest::default()
                    .from(fixture.sender)
                    .to(recipient)
                    .value(U256::from(1)),
            )
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        assert!(receipt.inner.logs().is_empty());
        let hash = receipt.block_hash.unwrap();
        api.mine_one().await.unwrap();
        let block = api.storage_info().block(hash).unwrap();
        assert!(block.header.logs_bloom.is_zero());
        assert!(block.body.transactions.is_empty());
        assert_ne!(block.header.transactions_root, alloy_consensus::constants::EMPTY_ROOT_HASH);
        assert_eq!(api.backend.mined_receipts(hash), Some(vec![]));

        let mut cx = Context::from_waker(Waker::noop());
        assert_eq!(logs.poll_next_unpin(&mut cx), Poll::Pending);

        let receipt = fixture.emit("matching notification").await;
        let notification = tokio::time::timeout(Duration::from_secs(5), logs.next())
            .await
            .expect("matching notification was not delivered")
            .expect("subscription ended before its matching block");
        assert_eq!(
            notification,
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "eth_subscription",
                "params": {"subscription": 1, "result": receipt.inner.logs()[0]},
            })
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lagged_logs_subscription_skips_bloom_excluded_pruned_blocks() {
        let fixture = LogsFixture::new().await;
        let api = &fixture.api;
        let sender = fixture._handle.dev_wallets().nth(1).unwrap().address();
        let provider = fixture.contract.provider();
        let address = EmitLogs::deploy_builder(provider.clone(), "other".to_owned())
            .from(sender)
            .deploy()
            .await
            .unwrap();
        let contract = EmitLogs::new(address, provider.clone());
        // Finish publishing setup notifications before installing the subscriptions.
        api.mine_one().await.unwrap();
        let mut subscriptions =
            [Filter::new().address(address), Filter::new().topic1(sender.into_word())].map(
                |filter| {
                    let subscription = fixture.logs(filter.clone());
                    (filter, subscription)
                },
            );

        let receipt = fixture.emit("excluded notification").await;
        let hash = receipt.block_hash.unwrap();
        api.mine_one().await.unwrap();
        let block = api.storage_info().block(hash).unwrap();
        assert!(!block.header.logs_bloom.is_zero());
        assert!(block.body.transactions.is_empty());
        assert_eq!(api.backend.mined_receipts(hash), Some(vec![]));

        let mut cx = Context::from_waker(Waker::noop());
        for (filter, subscription) in &mut subscriptions {
            assert!(!filter.matches_bloom(block.header.logs_bloom));
            assert_eq!(
                subscription.poll_next_unpin(&mut cx),
                Poll::Pending,
                "{filter:?} must skip bloom-excluded pruned blocks without ending the subscription"
            );
        }

        let receipt = contract
            .setValue("matching notification".to_owned())
            .from(sender)
            .send()
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        assert_eq!(receipt.inner.logs().len(), 1);
        for (_, subscription) in &mut subscriptions {
            let notification = tokio::time::timeout(Duration::from_secs(5), subscription.next())
                .await
                .expect("matching notification was not delivered")
                .expect("subscription ended before its matching block");
            assert_eq!(
                notification,
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "eth_subscription",
                    "params": {"subscription": 1, "result": receipt.inner.logs()[0]},
                })
            );
        }
    }

    #[cfg(unix)]
    mod ipc {
        use super::*;
        use anvil_rpc::response::ResponseResult;
        use anvil_server::{PubSubContext, PubSubRpcHandler, ipc::IpcEndpoint};
        use futures::{Stream, future::poll_fn, stream};
        use parking_lot::Mutex;
        use std::{
            pin::Pin,
            sync::{
                Arc,
                atomic::{AtomicBool, Ordering},
            },
        };
        use tokio::{
            io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
            net::UnixStream,
        };

        type LogStream = Pin<Box<dyn Stream<Item = serde_json::Value> + Send + Sync>>;

        #[derive(Clone)]
        struct LogsHandler {
            subscription: Arc<Mutex<Option<LogStream>>>,
        }

        impl PubSubRpcHandler for LogsHandler {
            type Request = serde_json::Value;
            type SubscriptionId = u64;
            type Subscription = LogStream;

            async fn on_request(
                &self,
                _request: Self::Request,
                cx: PubSubContext<Self>,
            ) -> ResponseResult {
                cx.add_subscription(1, self.subscription.lock().take().unwrap());
                ResponseResult::success(1)
            }

            fn close_on_subscription_end(&self, _subscription: &Self::Subscription) -> bool {
                true
            }
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn logs_survive_pruning_while_ipc_is_backpressured() {
            let fixture = LogsFixture::new().await;
            let api = &fixture.api;
            let logs = fixture.logs(Filter::new().address(*fixture.contract.address()));
            // Fill the unread socket before producing the log that will be pruned.
            let filler = serde_json::json!({"padding": "x".repeat(2 * 1024 * 1024)});
            let materialized = Arc::new(AtomicBool::new(false));
            let seen = materialized.clone();
            let subscription = stream::iter([filler.clone()]).chain(logs.inspect(move |_| {
                seen.store(true, Ordering::SeqCst);
            }));
            let handler =
                LogsHandler { subscription: Arc::new(Mutex::new(Some(Box::pin(subscription)))) };
            let pending_subscription = handler.subscription.clone();
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("logs.ipc");
            let endpoint = IpcEndpoint::new(handler, path.to_string_lossy().into_owned());
            let mut incoming = std::pin::pin!(endpoint.incoming().unwrap());
            let mut client = UnixStream::connect(path).await.unwrap();
            client
                .write_all(br#"{"jsonrpc":"2.0","method":"eth_subscribe","params":["logs"]}"#)
                .await
                .unwrap();
            let mut connection = Box::pin(incoming.next().await.unwrap());
            tokio::time::timeout(
                Duration::from_secs(5),
                poll_fn(|cx| {
                    assert!(connection.as_mut().poll(cx).is_pending());
                    if pending_subscription.lock().is_none() {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                }),
            )
            .await
            .expect("IPC subscription was not installed");

            let receipt = fixture.emit("retained notification").await;
            let block_hash = receipt.block_hash.unwrap();
            assert_eq!(api.backend.mined_receipts(block_hash).as_ref().map(Vec::len), Some(1));
            tokio::time::timeout(
                Duration::from_secs(5),
                poll_fn(|cx| {
                    assert!(connection.as_mut().poll(cx).is_pending());
                    if materialized.load(Ordering::SeqCst) {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                }),
            )
            .await
            .expect("log was not materialized before receipt pruning");

            api.mine_one().await.unwrap();
            assert_eq!(api.backend.mined_receipts(block_hash), Some(vec![]));

            let task = tokio::spawn(connection);
            let mut lines = BufReader::new(client).lines();
            for expected in [
                filler,
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "eth_subscription",
                    "params": {"subscription": 1, "result": receipt.inner.logs()[0]},
                }),
            ] {
                let notification = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
                    .await
                    .expect("IPC notification was lost while its receipts were pruned")
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&notification).unwrap(),
                    expected
                );
            }
            task.abort();
        }
    }
}
