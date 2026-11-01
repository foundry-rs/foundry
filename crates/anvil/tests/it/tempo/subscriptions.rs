//! Tests for Tempo consensus subscriptions.

use super::*;
use crate::utils::connect_pubsub;
use alloy_pubsub::Subscription;

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct FinalizedEvent {
    #[serde(rename = "type")]
    kind: FinalizedEventType,
    #[serde(flatten)]
    pub(super) certified: TempoCertifiedBlock,
    seen: u64,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
enum FinalizedEventType {
    #[serde(rename = "finalized")]
    Finalized,
}

pub(super) async fn subscribe(provider: &impl Provider) -> Subscription<FinalizedEvent> {
    let mut request = provider.client().request("consensus_subscribe", ());
    request.set_is_subscription();
    let id = tokio::time::timeout(Duration::from_secs(5), request).await.unwrap().unwrap();
    provider.root().get_subscription(id).await.unwrap()
}

pub(super) async fn next_event(subscription: &mut Subscription<FinalizedEvent>) -> FinalizedEvent {
    tokio::time::timeout(Duration::from_secs(5), subscription.recv_result())
        .await
        .expect("timed out waiting for a Tempo finalization")
        .expect("Tempo subscription ended unexpectedly")
        .expect("invalid Tempo finalization event")
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_subscription_broadcasts_complete_blocks_in_order() {
    let (api, handle) = spawn(
        NodeConfig::test_tempo()
            .with_no_mining(true)
            .with_transaction_block_keeper(Some(0usize))
            .with_tempo_epoch_length(NonZeroU64::new(3).unwrap()),
    )
    .await;
    let http = handle.http_provider();
    let identity = tempo_consensus_identity(&http).await;
    api.mine_one().await.unwrap();

    let first = connect_pubsub(&handle.ws_endpoint()).await;
    let second = connect_pubsub(&handle.ws_endpoint()).await;
    let mut first_events = subscribe(&first).await;
    let mut second_events = subscribe(&second).await;
    let rotated = tempo_rotate_identity(&http).await;
    assert_eq!(rotated.from_epoch, 1);
    assert_ne!(rotated.identity, identity.identity);
    let accounts = handle.dev_accounts().collect::<Vec<_>>();
    let transaction: B256 = http
        .client()
        .request(
            "eth_sendTransaction",
            (TransactionRequest::default()
                .from(accounts[0])
                .to(Address::repeat_byte(0x43))
                .gas_limit(1_000_000),),
        )
        .await
        .unwrap();
    let native_transaction: B256 = http
        .client()
        .request(
            "eth_sendTransaction",
            (tempo_call_request(accounts[1], [tempo_transfer(accounts[2], U256::ONE)]),),
        )
        .await
        .unwrap();

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

    for (expected, (epoch, view)) in expected.into_iter().zip([(0, 2), (1, 1), (1, 2)]) {
        let first_event = next_event(&mut first_events).await;
        let second_event = next_event(&mut second_events).await;
        assert_eq!(first_event, second_event);
        let certified = first_event.certified;
        assert_eq!(certified, expected);
        assert_eq!((certified.epoch, certified.view), (epoch, view));
        let signing_identity = if epoch == 0 { &identity } else { &rotated };
        assert_tempo_certificate(signing_identity, &certified);
        if certified.block.header.number() == 2 {
            let transactions = &certified.block.body.transactions;
            let mut transactions =
                transactions.iter().map(|tx| (tx.ty(), *tx.tx_hash())).collect::<Vec<_>>();
            transactions.sort_unstable();
            assert_eq!(transactions, [(2, transaction), (0x76, native_transaction)]);
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
    let state = api.anvil_dump_state(None).await.unwrap();
    assert!(api.anvil_load_state(state.clone()).await.unwrap());
    assert_eq!(tempo_finalization(&http, 1).await, expected[0]);
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

    assert!(api.anvil_load_state(state).await.unwrap());
    assert_eq!(http.get_block_number().await.unwrap(), 1);
    api.anvil_set_coinbase(Address::repeat_byte(0x34)).await.unwrap();
    api.mine_one().await.unwrap();
    expected.push(tempo_finalization(&http, 2).await);
    assert_ne!(expected[4].digest, expected[5].digest);

    api.anvil_reset(None).await.unwrap();
    api.mine_one().await.unwrap();
    expected.push(tempo_finalization(&http, 1).await);

    for certified in expected {
        let event = next_event(&mut events).await.certified;
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
    let consensus_fallback: String =
        provider.client().request("consensus_subscribe", ()).await.unwrap();
    let heads = provider.subscribe_blocks().await.unwrap();
    let heads_id = *heads.local_id();
    let mut heads = heads.into_stream();

    assert!(!other.client().request::<_, bool>("eth_unsubscribe", (&consensus,)).await.unwrap());
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
    assert!(
        provider
            .client()
            .request::<_, bool>("eth_unsubscribe", (&consensus_fallback,))
            .await
            .unwrap()
    );
    assert!(
        !provider
            .client()
            .request::<_, bool>("eth_unsubscribe", (&consensus_fallback,))
            .await
            .unwrap()
    );
    api.mine_one().await.unwrap();
    let head = tokio::time::timeout(Duration::from_secs(5), heads.next()).await.unwrap().unwrap();
    assert_eq!(head.number, 1);
    provider.unsubscribe(heads_id).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_subscription_rejects_non_tempo() {
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

    api.mine_one().await.unwrap();
    let event = next_event(&mut events).await.certified;
    assert_eq!(event, tempo_finalization(&http, 2).await);
    assert_tempo_certificate(&tempo_consensus_identity(&http).await, &event);
}
