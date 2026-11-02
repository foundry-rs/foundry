//! Tests for optional Tempo fork epoch metadata.

use super::*;
use std::sync::atomic::AtomicUsize;

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_fork_retains_intermittent_epoch_metadata() {
    let config = NodeConfig::test_tempo().with_no_mining(true);
    let (source_api, source) =
        spawn(config.clone().with_tempo_epoch_length(NonZeroU64::new(3).unwrap())).await;
    source_api.anvil_mine(Some(U256::from(3)), None).await.unwrap();
    let native =
        spawn_rpc_proxy_rejecting_method_after(source.http_endpoint(), "anvil_nodeInfo", 0).await;
    let (native, _) = spawn_rpc_proxy_canned_method(
        native,
        "debug_chainConfig",
        serde_json::json!({ "epochLength": 3 }),
    )
    .await;
    for (endpoint, method) in
        [(source.http_endpoint(), "anvil_tempoConsensusIdentity"), (native, "debug_chainConfig")]
    {
        for missing_first in [false, true] {
            let endpoint = alternating_epoch_metadata(
                endpoint.clone(),
                method,
                serde_json::Value::Null,
                missing_first,
            )
            .await;
            let (api, handle) = spawn(
                config.clone().with_eth_rpc_url(Some(endpoint)).with_fork_block_number(Some(3u64)),
            )
            .await;
            let provider = handle.http_provider();
            let identity = tempo_consensus_identity(&provider).await;
            assert_eq!(identity.epoch_length, 3);
            api.mine_one().await.unwrap();
            assert_tempo_fork_round(&provider, &identity, 4, (1, 2, 1)).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_fork_intermittent_epoch_mirror_replacement_and_reset() {
    let config = NodeConfig::test_tempo().with_no_mining(true);
    let (source_api, source) =
        spawn(config.clone().with_tempo_epoch_length(NonZeroU64::new(3).unwrap())).await;
    source_api.anvil_mine(Some(U256::from(3)), None).await.unwrap();
    let hidden = spawn_rpc_proxy_rejecting_method_after(
        source.http_endpoint(),
        "anvil_tempoConsensusIdentity",
        0,
    )
    .await;
    for missing_first in [false, true] {
        let endpoint = alternating_epoch_metadata(
            source.http_endpoint(),
            "anvil_tempoConsensusIdentity",
            serde_json::Value::Null,
            missing_first,
        )
        .await;
        let (api, handle) = spawn(
            config
                .clone()
                .with_fork_urls(vec![hidden.clone(), endpoint.clone()])
                .with_fork_block_number(Some(1u64)),
        )
        .await;
        let provider = handle.http_provider();
        let identity = tempo_consensus_identity(&provider).await;
        assert_eq!(identity.epoch_length, 3);
        api.anvil_set_rpc_url(endpoint.clone()).await.unwrap();
        assert_eq!(tempo_consensus_identity(&provider).await, identity);
        api.mine_one().await.unwrap();
        assert_tempo_fork_round(&provider, &identity, 2, (0, 2, 1)).await;

        api.anvil_reset(Some(Forking { json_rpc_url: Some(endpoint), block_number: Some(3) }))
            .await
            .unwrap();
        let reset_identity = tempo_consensus_identity(&provider).await;
        assert_eq!(reset_identity.epoch_length, 3);
        assert_eq!(reset_identity.identity, identity.identity);
        assert_eq!(reset_identity.from_epoch, 1);
        api.mine_one().await.unwrap();
        assert_tempo_fork_round(&provider, &reset_identity, 4, (1, 2, 1)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_fork_rejects_conflicting_epoch_observations() {
    let config = NodeConfig::test_tempo().with_no_mining(true);
    let (source_api, source) =
        spawn(config.clone().with_tempo_epoch_length(NonZeroU64::new(3).unwrap())).await;
    source_api.mine_one().await.unwrap();
    let endpoint = alternating_epoch_metadata(
        source.http_endpoint(),
        "anvil_tempoConsensusIdentity",
        serde_json::json!({ "epochLength": 6 }),
        false,
    )
    .await;
    let error = try_spawn(
        config.clone().with_eth_rpc_url(Some(endpoint.clone())).with_fork_block_number(Some(1u64)),
    )
    .await
    .err()
    .expect("conflicting known epoch lengths must not establish a stable fork");
    assert_eq!(
        error.to_string(),
        "fork endpoint changed while its identity and block context were being resolved"
    );

    let (api, handle) = spawn(
        config.with_eth_rpc_url(Some(source.http_endpoint())).with_fork_block_number(Some(1u64)),
    )
    .await;
    let provider = handle.http_provider();
    api.mine_one().await.unwrap();
    let identity = tempo_consensus_identity(&provider).await;
    let finalized = tempo_finalization(&provider, 2).await;
    let error = api.anvil_set_rpc_url(endpoint).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "fork endpoint changed while its identity and active fork block were being resolved"
    );
    assert_eq!(tempo_consensus_identity(&provider).await, identity);
    assert_eq!(tempo_finalization(&provider, 2).await, finalized);
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_fork_epoch_ignores_unrelated_native_configuration() {
    let config = NodeConfig::test_tempo().with_no_mining(true);
    let (source_api, source) =
        spawn(config.clone().with_tempo_epoch_length(NonZeroU64::new(3).unwrap())).await;
    source_api.mine_one().await.unwrap();
    let native =
        spawn_rpc_proxy_rejecting_method_after(source.http_endpoint(), "anvil_nodeInfo", 0).await;
    for metadata in [
        serde_json::json!({ "epochLength": 3, "chainId": "0x1" }),
        serde_json::json!({ "epochLength": 3, "depositContractAddress": "not-an-address" }),
    ] {
        let (endpoint, _) =
            spawn_rpc_proxy_canned_method(native.clone(), "debug_chainConfig", metadata).await;
        let (api, handle) = spawn(
            config.clone().with_eth_rpc_url(Some(endpoint)).with_fork_block_number(Some(1u64)),
        )
        .await;
        let provider = handle.http_provider();
        let identity = tempo_consensus_identity(&provider).await;
        assert_eq!(identity.epoch_length, 3);
        api.mine_one().await.unwrap();
        assert_tempo_fork_round(&provider, &identity, 2, (0, 2, 1)).await;
    }
}

async fn alternating_epoch_metadata(
    endpoint: String,
    method: &'static str,
    alternate: serde_json::Value,
    alternate_first: bool,
) -> String {
    spawn_rpc_proxy_mapping_method(endpoint, method, {
        let calls = AtomicUsize::new(0);
        move |_, value| {
            if calls.fetch_add(1, Ordering::Relaxed).is_multiple_of(2) == alternate_first {
                alternate.clone()
            } else {
                value
            }
        }
    })
    .await
}
