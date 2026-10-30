//! Tests for Tempo consensus subscriptions.

use super::*;
use crate::utils::connect_pubsub;
use alloy_pubsub::Subscription;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct FinalizedEvent {
    #[serde(rename = "type")]
    kind: FinalizedEventType,
    epoch: u64,
    view: u64,
    digest: B256,
    certificate: String,
    block: tempo_primitives::Block,
    seen: u64,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
enum FinalizedEventType {
    #[serde(rename = "finalized")]
    Finalized,
}

impl FinalizedEvent {
    fn into_certified_block(self) -> TempoCertifiedBlock {
        TempoCertifiedBlock {
            epoch: self.epoch,
            view: self.view,
            digest: self.digest,
            certificate: self.certificate,
            block: self.block,
        }
    }
}

async fn subscribe(provider: &impl Provider) -> Subscription<FinalizedEvent> {
    let mut request = provider.client().request("consensus_subscribe", ());
    request.set_is_subscription();
    let id = tokio::time::timeout(Duration::from_secs(5), request).await.unwrap().unwrap();
    provider.root().get_subscription(id).await.unwrap()
}

async fn next_event(subscription: &mut Subscription<FinalizedEvent>) -> FinalizedEvent {
    tokio::time::timeout(Duration::from_secs(5), subscription.recv_result())
        .await
        .expect("timed out waiting for a Tempo finalization")
        .expect("Tempo subscription ended unexpectedly")
        .expect("invalid Tempo finalization event")
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_subscription_broadcasts_complete_blocks_in_order() {
    let (api, handle) = spawn(
        NodeConfig::test_tempo().with_no_mining(true).with_transaction_block_keeper(Some(0usize)),
    )
    .await;
    let http = handle.http_provider();
    let identity = tempo_consensus_identity(&http).await;
    api.mine_one().await.unwrap();

    let first = connect_pubsub(&handle.ws_endpoint()).await;
    let second = connect_pubsub(&handle.ws_endpoint()).await;
    let mut first_events = subscribe(&first).await;
    let mut second_events = subscribe(&second).await;
    let transaction: B256 = http
        .client()
        .request(
            "eth_sendTransaction",
            (TransactionRequest::default()
                .from(handle.dev_accounts().next().unwrap())
                .to(Address::repeat_byte(0x43))
                .gas_limit(1_000_000),),
        )
        .await
        .unwrap();

    let before = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
    let mut expected = Vec::new();
    for _ in 2..=4 {
        api.mine_one().await.unwrap();
        expected.push(
            http.client()
                .request::<_, TempoCertifiedBlock>("consensus_getFinalization", ("latest",))
                .await
                .unwrap(),
        );
    }
    assert_tempo_missing(&http, serde_json::json!({ "height": 2 }), "block").await;

    for expected in expected {
        let first_event = next_event(&mut first_events).await;
        let second_event = next_event(&mut second_events).await;
        let after = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
        assert_eq!(first_event, second_event);
        assert!((before..=after).contains(&u128::from(first_event.seen)));
        let certified = first_event.into_certified_block();
        assert_eq!(certified, expected);
        assert_tempo_certificate(&identity, &certified);
        if certified.block.header.number() == 2 {
            assert_eq!(certified.block.body.transactions.len(), 1);
            assert_eq!(*certified.block.body.transactions[0].tx_hash(), transaction);
        } else {
            assert!(certified.block.body.transactions.is_empty());
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_subscription_preserves_events_across_history_changes() {
    let (api, handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
    let http = handle.http_provider();
    let identity = tempo_consensus_identity(&http).await;
    let provider = connect_pubsub(&handle.ws_endpoint()).await;
    let mut events = subscribe(&provider).await;
    let mut expected = Vec::new();

    api.mine_one().await.unwrap();
    expected.push(tempo_finalization(&http, 1).await);
    let snapshot = api.evm_snapshot().await.unwrap();
    api.mine_one().await.unwrap();
    expected.push(tempo_finalization(&http, 2).await);

    assert!(api.evm_revert(snapshot).await.unwrap());
    api.anvil_set_coinbase(Address::repeat_byte(0x31)).await.unwrap();
    api.mine_one().await.unwrap();
    expected.push(tempo_finalization(&http, 2).await);
    assert_ne!(expected[1].digest, expected[2].digest);

    api.anvil_rollback(Some(1)).await.unwrap();
    api.anvil_set_coinbase(Address::repeat_byte(0x32)).await.unwrap();
    api.mine_one().await.unwrap();
    expected.push(tempo_finalization(&http, 2).await);
    assert_ne!(expected[2].digest, expected[3].digest);

    api.anvil_set_coinbase(Address::repeat_byte(0x33)).await.unwrap();
    api.anvil_reorg(ReorgOptions { depth: 1, tx_block_pairs: Vec::new() }).await.unwrap();
    expected.push(tempo_finalization(&http, 2).await);
    assert_ne!(expected[3].digest, expected[4].digest);

    api.anvil_reset(None).await.unwrap();
    api.mine_one().await.unwrap();
    expected.push(tempo_finalization(&http, 1).await);

    for certified in expected {
        let event = next_event(&mut events).await.into_certified_block();
        assert_eq!(event, certified);
        assert_tempo_certificate(&identity, &event);
    }
    assert!(tokio::time::timeout(Duration::from_millis(100), events.recv_result()).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_subscription_unsubscribe_is_scoped() {
    let (api, handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
    let provider = connect_pubsub(&handle.ws_endpoint()).await;
    let other = connect_pubsub(&handle.ws_endpoint()).await;
    let consensus: String = provider.client().request("consensus_subscribe", ()).await.unwrap();
    let heads = provider.subscribe_blocks().await.unwrap();
    let heads_id = *heads.local_id();
    let mut heads = heads.into_stream();

    assert!(!provider.client().request::<_, bool>("eth_unsubscribe", (&consensus,)).await.unwrap());
    assert!(
        !other.client().request::<_, bool>("consensus_unsubscribe", (&consensus,)).await.unwrap()
    );
    assert!(
        provider.client().request::<_, bool>("consensus_unsubscribe", (&consensus,)).await.unwrap()
    );
    assert!(
        !provider
            .client()
            .request::<_, bool>("consensus_unsubscribe", (&consensus,))
            .await
            .unwrap()
    );
    api.mine_one().await.unwrap();
    let head = tokio::time::timeout(Duration::from_secs(5), heads.next()).await.unwrap().unwrap();
    assert_eq!(head.number, 1);
    provider.unsubscribe(heads_id).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_subscription_fork_only_publishes_local_finalizations() {
    let config = NodeConfig::test_tempo().with_no_mining(true);
    let (source_api, source) = spawn(config.clone()).await;
    source_api.mine_one().await.unwrap();
    let source_http = source.http_provider();
    let (api, handle) = spawn(
        tempo_fork_config(config, &source).with_tempo_consensus_seed(Some(B256::repeat_byte(0x42))),
    )
    .await;
    let http = handle.http_provider();
    let identity = tempo_consensus_identity(&http).await;
    assert_ne!(identity, tempo_consensus_identity(&source_http).await);
    let provider = connect_pubsub(&handle.ws_endpoint()).await;
    let mut events = subscribe(&provider).await;

    source_api.mine_one().await.unwrap();
    let latest: TempoCertifiedBlock =
        http.client().request("consensus_getFinalization", ("latest",)).await.unwrap();
    assert_eq!(latest, tempo_finalization(&source_http, 1).await);

    api.mine_one().await.unwrap();
    let event = next_event(&mut events).await.into_certified_block();
    assert_eq!(event, tempo_finalization(&http, 2).await);
    assert_tempo_certificate(&identity, &event);

    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(source.http_endpoint()),
        block_number: Some(2),
    }))
    .await
    .unwrap();
    assert_eq!(tempo_finalization(&http, 2).await, tempo_finalization(&source_http, 2).await);
    api.mine_one().await.unwrap();
    let event = next_event(&mut events).await.into_certified_block();
    assert_eq!(event, tempo_finalization(&http, 3).await);
    assert_tempo_certificate(&identity, &event);
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_subscription_rejects_non_tempo_and_invalid_params() {
    let (_api, handle) = spawn(NodeConfig::test().with_no_mining(true)).await;
    let provider = connect_pubsub(&handle.ws_endpoint()).await;
    let error =
        provider.client().request::<_, String>("consensus_subscribe", ()).await.unwrap_err();
    assert_eq!(
        serde_json::to_value(error.as_error_resp().unwrap()).unwrap(),
        serde_json::json!({
            "code": -32602,
            "message": "Tempo consensus is only available on the Tempo network",
            "data": null,
        }),
    );

    for (method, params, message) in [
        ("consensus_subscribe", serde_json::json!([1]), "invalid type: number, expected unit"),
        (
            "consensus_unsubscribe",
            serde_json::json!([]),
            "expected params sequence with length 1 but got 0",
        ),
    ] {
        let error =
            provider.client().request::<_, serde_json::Value>(method, params).await.unwrap_err();
        assert_eq!(
            serde_json::to_value(error.as_error_resp().unwrap()).unwrap(),
            serde_json::json!({
                "code": -32602,
                "message": message,
                "data": null,
            }),
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_subscription_skips_non_native_block_without_stopping_mining() {
    let (api, handle) =
        spawn(NodeConfig::test_tempo().with_no_mining(true).with_auto_impersonate(true)).await;
    let http = handle.http_provider();
    let provider = connect_pubsub(&handle.ws_endpoint()).await;
    let mut events = subscribe(&provider).await;
    let from = handle.dev_accounts().next().unwrap();
    let gas_price = http.get_gas_price().await.unwrap();
    let blob_transaction: B256 = http
        .client()
        .request(
            "eth_sendTransaction",
            (TransactionRequest {
                from: Some(from),
                to: Some(TxKind::Call(Address::repeat_byte(0x43))),
                nonce: Some(0),
                gas: Some(1_000_000),
                max_fee_per_gas: Some(gas_price),
                max_priority_fee_per_gas: Some(0),
                max_fee_per_blob_gas: Some(gas_price + 1),
                blob_versioned_hashes: Some(vec![B256::repeat_byte(1)]),
                transaction_type: Some(3),
                ..Default::default()
            },),
        )
        .await
        .unwrap();

    api.mine_one().await.unwrap();
    let receipt = http.get_transaction_receipt(blob_transaction).await.unwrap().unwrap();
    assert!(receipt.status());
    assert_eq!(receipt.block_number, Some(1));

    api.anvil_auto_impersonate_account(false).await.unwrap();
    let transaction: B256 = http
        .client()
        .request(
            "eth_sendTransaction",
            (TransactionRequest::default()
                .from(from)
                .to(Address::repeat_byte(0x43))
                .gas_limit(1_000_000),),
        )
        .await
        .unwrap();
    api.mine_one().await.unwrap();
    let event = next_event(&mut events).await.into_certified_block();
    assert_eq!(event, tempo_finalization(&http, 2).await);
    assert_eq!(event.block.body.transactions.len(), 1);
    assert_eq!(*event.block.body.transactions[0].tx_hash(), transaction);
    assert_tempo_certificate(&tempo_consensus_identity(&http).await, &event);
}
