//! Inconclusive endpoint identity must not enable BAL prefill of mutable state.

use super::{BalOrigin, BalProxy, BalResponse, CONTRACT};
use alloy_primitives::{B256, U256};
use alloy_rpc_types::anvil::Forking;
use anvil::spawn;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_skips_inconclusive_node_identity() {
    let origin = BalOrigin::new().await;
    origin
        .api
        .anvil_set_storage_at(CONTRACT, U256::ZERO, B256::from(U256::from(99)))
        .await
        .unwrap();

    for mode in [
        BalResponse::NodeInfoTimeout,
        BalResponse::NodeInfoTimeoutOnce,
        BalResponse::NodeInfoInternalError,
        BalResponse::NodeInfoMalformed,
        BalResponse::MetadataTimeout,
    ] {
        for no_bal in [false, true] {
            let proxy =
                BalProxy::new(&origin.handle, mode, matches!(mode, BalResponse::MetadataTimeout))
                    .await;
            let (api, _handle) = tokio::time::timeout(
                Duration::from_secs(5),
                spawn(
                    origin
                        .config(&proxy)
                        .with_no_bal(no_bal)
                        .fork_request_timeout(Some(Duration::from_secs(60))),
                ),
            )
            .await
            .unwrap_or_else(|_| panic!("identity failure must not prevent fork startup: {mode:?}"));
            assert_eq!(
                api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
                B256::from(U256::from(99)),
                "{mode:?}, no_bal={no_bal}",
            );
            assert_eq!(proxy.count("eth_getBlockAccessListByBlockHash"), 0, "{mode:?}");
            assert_eq!(proxy.count("eth_getStorageAt"), 1, "{mode:?}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_skips_inconclusive_identity_revalidation() {
    let origin = BalOrigin::new().await;
    origin
        .api
        .anvil_set_storage_at(CONTRACT, U256::ZERO, B256::from(U256::from(99)))
        .await
        .unwrap();
    for (primary_mode, mirror_mode) in [
        (BalResponse::NodeInfoInternalErrorAfterDiscovery, BalResponse::Valid),
        (BalResponse::Valid, BalResponse::NodeInfoInternalError),
    ] {
        let primary = BalProxy::new(&origin.handle, primary_mode, false).await;
        let mirror = BalProxy::new(&origin.handle, mirror_mode, false).await;
        let (api, _handle) = spawn(
            origin
                .config(&primary)
                .with_fork_urls(vec![primary.endpoint.clone(), mirror.endpoint.clone()]),
        )
        .await;
        assert_eq!(
            api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
            B256::from(U256::from(99))
        );
        assert_eq!(primary.count("eth_getBlockAccessListByBlockHash"), 0);
        assert_eq!(mirror.count("eth_getBlockAccessListByBlockHash"), 0);
        assert_eq!(primary.count("eth_getStorageAt") + mirror.count("eth_getStorageAt"), 1);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_reset_skips_inconclusive_node_identity() {
    let origin = BalOrigin::new().await;
    let proxy = BalProxy::new(&origin.handle, BalResponse::Valid, false).await;
    let (api, _handle) = spawn(origin.config(&proxy)).await;
    assert_eq!(api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(), B256::from(U256::ONE));
    assert_eq!(proxy.count("eth_getBlockAccessListByBlockHash"), 1);
    assert_eq!(proxy.count("eth_getStorageAt"), 0);

    origin
        .api
        .anvil_set_storage_at(CONTRACT, U256::ZERO, B256::from(U256::from(99)))
        .await
        .unwrap();
    let replacement =
        BalProxy::new(&origin.handle, BalResponse::NodeInfoInternalError, false).await;
    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(replacement.endpoint.clone()),
        block_number: Some(origin.block_number),
    }))
    .await
    .unwrap();
    assert_eq!(
        api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
        B256::from(U256::from(99))
    );
    assert_eq!(replacement.count("eth_getBlockAccessListByBlockHash"), 0);
    assert_eq!(replacement.count("eth_getStorageAt"), 1);
}
