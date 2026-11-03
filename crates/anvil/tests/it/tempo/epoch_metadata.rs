//! Tests for optional Tempo fork epoch metadata.

use super::*;
use std::sync::atomic::AtomicUsize;

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_fork_retains_intermittent_epoch_metadata() {
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
    for (method, endpoint) in tempo_epoch_metadata_endpoints(&source).await {
        let endpoint = if method == "debug_chainConfig" {
            spawn_rpc_proxy_canned_method(endpoint, method, serde_json::json!({ "epochLength": 3 }))
                .await
                .0
        } else {
            endpoint
        };
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

            if method == "anvil_tempoConsensusIdentity" {
                let endpoint = alternating_epoch_metadata(
                    source.http_endpoint(),
                    method,
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

                api.anvil_reset(Some(Forking {
                    json_rpc_url: Some(endpoint),
                    block_number: Some(3),
                }))
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

    let (api, handle) = spawn(tempo_fork_config(config, &source)).await;
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

/// Reveals metadata after the pinned fork root is fetched.
/// The returned counter tracks full root reads so tests can detect repeated initialization.
pub(super) async fn epoch_metadata_after_fork_root(
    endpoint: String,
    block_number: u64,
) -> (String, Arc<AtomicUsize>) {
    let root_reads = Arc::new(AtomicUsize::new(0));
    let mut endpoint = spawn_rpc_proxy_mapping_method(endpoint, "anvil_tempoConsensusIdentity", {
        let root_reads = Arc::clone(&root_reads);
        move |_, value| {
            if root_reads.load(Ordering::Relaxed) == 0 { serde_json::Value::Null } else { value }
        }
    })
    .await;
    let root_number = serde_json::to_value(BlockNumberOrTag::Number(block_number)).unwrap();
    for method in ["eth_getBlockByNumber", "eth_getBlockByHash"] {
        endpoint = spawn_rpc_proxy_mapping_method(endpoint, method, {
            let root_reads = Arc::clone(&root_reads);
            let root_number = root_number.clone();
            move |params, value| {
                if params.get(1).and_then(serde_json::Value::as_bool) == Some(true)
                    && value.get("number") == Some(&root_number)
                {
                    root_reads.fetch_add(1, Ordering::Relaxed);
                }
                value
            }
        })
        .await;
    }
    (endpoint, root_reads)
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_fork_accepts_late_epoch_metadata_with_unchanged_schedule() {
    let config = NodeConfig::test_tempo().with_no_mining(true);
    for (source_length, override_length) in [(3, Some(3)), (6, Some(3)), (21_600, None)] {
        let (source_api, source) =
            spawn(config.clone().with_tempo_epoch_length(NonZeroU64::new(source_length).unwrap()))
                .await;
        source_api.mine_one().await.unwrap();
        let (endpoint, root_reads) =
            epoch_metadata_after_fork_root(source.http_endpoint(), 1).await;
        let mut fork_config =
            config.clone().with_eth_rpc_url(Some(endpoint)).with_fork_block_number(Some(1u64));
        if let Some(length) = override_length {
            fork_config = fork_config.with_tempo_epoch_length(NonZeroU64::new(length).unwrap());
        }
        let (api, handle) = try_spawn(fork_config)
            .await
            .expect("late source metadata must not reject an unchanged consensus schedule");
        let provider = handle.http_provider();
        let identity = tempo_consensus_identity(&provider).await;
        assert_eq!(identity.epoch_length, override_length.unwrap_or(source_length));
        assert_eq!(root_reads.load(Ordering::Relaxed), 1);

        let (conflicting, _) = spawn_rpc_proxy_canned_method(
            source.http_endpoint(),
            "anvil_tempoConsensusIdentity",
            serde_json::json!({ "epochLength": 9 }),
        )
        .await;
        for reset in [false, true] {
            if reset {
                let (endpoint, root_reads) =
                    epoch_metadata_after_fork_root(source.http_endpoint(), 1).await;
                api.anvil_reset(Some(Forking {
                    json_rpc_url: Some(endpoint),
                    block_number: Some(1),
                }))
                .await
                .unwrap();
                assert_eq!(root_reads.load(Ordering::Relaxed), 1);
                assert_eq!(tempo_consensus_identity(&provider).await, identity);
            }
            let error = api.anvil_set_rpc_url(conflicting.clone()).await.unwrap_err();
            assert_eq!(
                error.to_string(),
                "replacement fork endpoint has an incompatible execution context"
            );
            assert_eq!(tempo_consensus_identity(&provider).await, identity);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_fork_rejects_late_epoch_metadata_that_changes_schedule() {
    let config = NodeConfig::test_tempo().with_no_mining(true);
    let (source_api, source) =
        spawn(config.clone().with_tempo_epoch_length(NonZeroU64::new(3).unwrap())).await;
    source_api.mine_one().await.unwrap();
    let (endpoint, root_reads) = epoch_metadata_after_fork_root(source.http_endpoint(), 1).await;
    let error =
        try_spawn(config.with_eth_rpc_url(Some(endpoint)).with_fork_block_number(Some(1u64)))
            .await
            .err()
            .expect(
                "late source metadata must reject consensus built with a different epoch length",
            );
    assert_eq!(error.to_string(), "fork endpoint changed while Anvil was being initialized");
    assert_eq!(root_reads.load(Ordering::Relaxed), 1);
}
