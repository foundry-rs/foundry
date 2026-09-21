//! Inconclusive endpoint identity must not enable BAL prefill of mutable state.

use super::{BalOrigin, BalProxy, BalResponse, CONTRACT};
use alloy_primitives::{B256, U256};
use alloy_rpc_types::anvil::Forking;
use anvil::{spawn, try_spawn};
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::task::JoinHandle;

#[derive(Clone, Copy, Debug)]
enum IdentityAfterBal {
    Unsupported,
    InternalError,
    InternalErrorAfterValidation,
    Timeout,
    Anvil,
}

/// Changes identity discovery only after the primary has returned a BAL.
struct IdentityProxy {
    endpoint: String,
    post_fetch_probes: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl IdentityProxy {
    async fn new(upstream: String, mode: IdentityAfterBal, bal_returned: Arc<AtomicBool>) -> Self {
        let client = reqwest::Client::new();
        let post_fetch_probes = Arc::new(AtomicUsize::new(0));
        let probes = Arc::clone(&post_fetch_probes);
        let router = Router::new().route(
            "/",
            post(move |Json(request): Json<Value>| {
                let upstream = upstream.clone();
                let client = client.clone();
                let bal_returned = Arc::clone(&bal_returned);
                let probes = Arc::clone(&probes);
                async move {
                    let method = request["method"].as_str().unwrap();
                    if method == "anvil_nodeInfo" {
                        let mode = if bal_returned.load(Ordering::SeqCst) {
                            let previous = probes.fetch_add(1, Ordering::SeqCst);
                            if matches!(mode, IdentityAfterBal::InternalErrorAfterValidation)
                                && previous < 2
                            {
                                IdentityAfterBal::Unsupported
                            } else {
                                mode
                            }
                        } else {
                            IdentityAfterBal::Unsupported
                        };
                        match mode {
                            IdentityAfterBal::Unsupported
                            | IdentityAfterBal::InternalError
                            | IdentityAfterBal::InternalErrorAfterValidation => {
                                let code = if matches!(mode, IdentityAfterBal::Unsupported) {
                                    -32601
                                } else {
                                    -32603
                                };
                                return Json(json!({
                                    "jsonrpc": "2.0", "id": request["id"],
                                    "error": {"code": code, "message": "injected identity failure"},
                                }));
                            }
                            IdentityAfterBal::Timeout => return futures::future::pending().await,
                            IdentityAfterBal::Anvil => {}
                        }
                    }
                    let response = client
                        .post(upstream)
                        .json(&request)
                        .send()
                        .await
                        .unwrap()
                        .json::<Value>()
                        .await
                        .unwrap();
                    if matches!(
                        method,
                        "eth_getBlockAccessList" | "eth_getBlockAccessListByBlockHash"
                    ) && response["result"].is_array()
                    {
                        bal_returned.store(true, Ordering::SeqCst);
                    }
                    Json(response)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self { endpoint, post_fetch_probes, task }
    }
}

impl Drop for IdentityProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_skips_identity_failure_after_fetch() {
    let origin = BalOrigin::new().await;
    origin
        .api
        .anvil_set_storage_at(CONTRACT, U256::ZERO, B256::from(U256::from(99)))
        .await
        .unwrap();

    for (mode, multiple_urls, failing_mirror) in [
        (IdentityAfterBal::InternalError, false, false),
        (IdentityAfterBal::Timeout, false, false),
        (IdentityAfterBal::InternalError, true, false),
        (IdentityAfterBal::InternalError, true, true),
    ] {
        let proxy = BalProxy::new(&origin.handle, BalResponse::Valid, true).await;
        let bal_returned = Arc::new(AtomicBool::new(false));
        let primary = IdentityProxy::new(
            proxy.endpoint.clone(),
            if failing_mirror { IdentityAfterBal::Unsupported } else { mode },
            Arc::clone(&bal_returned),
        )
        .await;
        let mirror = IdentityProxy::new(
            proxy.endpoint.clone(),
            if failing_mirror { mode } else { IdentityAfterBal::Unsupported },
            bal_returned,
        )
        .await;
        let mut urls = vec![primary.endpoint.clone()];
        if multiple_urls {
            urls.push(mirror.endpoint.clone());
        }
        let (api, _handle) = tokio::time::timeout(
            Duration::from_secs(5),
            spawn(origin.config(&proxy).with_fork_urls(urls)),
        )
        .await
        .expect("post-fetch identity failure must preserve bounded lazy fallback");
        assert_eq!(proxy.count("eth_getBlockAccessList"), 1);
        assert_eq!(
            api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
            B256::from(U256::from(99)),
            "{mode:?}, multiple_urls={multiple_urls}, failing_mirror={failing_mirror}",
        );
        assert_eq!(proxy.count("eth_getStorageAt"), 1);
        let failing = if failing_mirror { &mirror } else { &primary };
        assert!(failing.post_fetch_probes.load(Ordering::SeqCst) > 0);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_rejects_identity_change_after_fetch() {
    let origin = BalOrigin::new().await;
    for (multiple_urls, changed_mirror) in [(false, false), (true, false), (true, true)] {
        let proxy = BalProxy::new(&origin.handle, BalResponse::Valid, true).await;
        let bal_returned = Arc::new(AtomicBool::new(false));
        let primary = IdentityProxy::new(
            proxy.endpoint.clone(),
            if changed_mirror { IdentityAfterBal::Unsupported } else { IdentityAfterBal::Anvil },
            Arc::clone(&bal_returned),
        )
        .await;
        let mirror = IdentityProxy::new(
            proxy.endpoint.clone(),
            if changed_mirror { IdentityAfterBal::Anvil } else { IdentityAfterBal::Unsupported },
            bal_returned,
        )
        .await;
        let mut urls = vec![primary.endpoint.clone()];
        if multiple_urls {
            urls.push(mirror.endpoint.clone());
        }
        let result = try_spawn(origin.config(&proxy).with_fork_urls(urls)).await;
        let Err(error) = result else {
            panic!("expected changed fork identity to reject the seed")
        };
        assert!(
            error
                .to_string()
                .contains("fork endpoint changed while its block access list was being fetched"),
            "{error}"
        );
        assert_eq!(proxy.count("eth_getBlockAccessList"), 1);
        let changed = if changed_mirror { &mirror } else { &primary };
        assert!(changed.post_fetch_probes.load(Ordering::SeqCst) > 0);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_rejects_identity_failure_after_seed_application() {
    let origin = BalOrigin::new().await;
    for (multiple_urls, failing_mirror) in [(false, false), (true, false), (true, true)] {
        let proxy = BalProxy::new(&origin.handle, BalResponse::Valid, true).await;
        let bal_returned = Arc::new(AtomicBool::new(false));
        let primary = IdentityProxy::new(
            proxy.endpoint.clone(),
            if failing_mirror {
                IdentityAfterBal::Unsupported
            } else {
                IdentityAfterBal::InternalErrorAfterValidation
            },
            Arc::clone(&bal_returned),
        )
        .await;
        let mirror = IdentityProxy::new(
            proxy.endpoint.clone(),
            if failing_mirror {
                IdentityAfterBal::InternalErrorAfterValidation
            } else {
                IdentityAfterBal::Unsupported
            },
            bal_returned,
        )
        .await;
        let mut urls = vec![primary.endpoint.clone()];
        if multiple_urls {
            urls.push(mirror.endpoint.clone());
        }
        let result = try_spawn(origin.config(&proxy).with_fork_urls(urls)).await;
        let Err(error) = result else { panic!("expected inconclusive identity to reject startup") };
        assert!(
            error.to_string().contains("fork endpoint changed while Anvil was being initialized"),
            "{error}"
        );
        assert_eq!(proxy.count("eth_getBlockAccessList"), 1);
        let failing = if failing_mirror { &mirror } else { &primary };
        assert!(failing.post_fetch_probes.load(Ordering::SeqCst) > 2);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_reset_revalidates_identity_after_fetch() {
    for mode in [
        IdentityAfterBal::InternalError,
        IdentityAfterBal::Anvil,
        IdentityAfterBal::InternalErrorAfterValidation,
    ] {
        let origin = BalOrigin::new().await;
        let proxy = BalProxy::new(&origin.handle, BalResponse::Valid, false).await;
        let (api, _handle) = spawn(origin.config(&proxy)).await;
        let snapshot = api.evm_snapshot().await.unwrap();
        api.anvil_set_storage_at(CONTRACT, U256::ZERO, B256::from(U256::from(77))).await.unwrap();
        origin
            .api
            .anvil_set_storage_at(CONTRACT, U256::ZERO, B256::from(U256::from(99)))
            .await
            .unwrap();
        let replacement = BalProxy::new(&origin.handle, BalResponse::Valid, true).await;
        let changing = IdentityProxy::new(
            replacement.endpoint.clone(),
            mode,
            Arc::new(AtomicBool::new(false)),
        )
        .await;
        let result = api
            .anvil_reset(Some(Forking {
                json_rpc_url: Some(changing.endpoint.clone()),
                block_number: Some(origin.block_number),
            }))
            .await;
        assert_eq!(replacement.count("eth_getBlockAccessList"), 1);
        assert!(changing.post_fetch_probes.load(Ordering::SeqCst) > 0);
        if matches!(mode, IdentityAfterBal::Anvil) {
            let error = result.unwrap_err();
            assert!(
                error.to_string().contains(
                    "fork endpoint changed while its block access list was being fetched"
                ),
                "{error}"
            );
            assert_eq!(
                api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
                B256::from(U256::from(77))
            );
            assert!(api.evm_revert(snapshot).await.unwrap());
            assert_eq!(
                api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
                B256::from(U256::ONE)
            );
        } else {
            result.unwrap();
            if matches!(mode, IdentityAfterBal::InternalErrorAfterValidation) {
                assert!(changing.post_fetch_probes.load(Ordering::SeqCst) >= 6);
            }
            assert_eq!(
                api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
                B256::from(U256::from(99))
            );
            assert_eq!(replacement.count("eth_getStorageAt"), 1);
        }
    }
}

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
            assert_eq!(proxy.count("eth_getBlockAccessList"), 0, "{mode:?}");
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
        assert_eq!(primary.count("eth_getBlockAccessList"), 0);
        assert_eq!(mirror.count("eth_getBlockAccessList"), 0);
        assert_eq!(primary.count("eth_getStorageAt") + mirror.count("eth_getStorageAt"), 1);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_reset_skips_inconclusive_node_identity() {
    let origin = BalOrigin::new().await;
    let proxy = BalProxy::new(&origin.handle, BalResponse::Valid, false).await;
    let (api, _handle) = spawn(origin.config(&proxy)).await;
    assert_eq!(api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(), B256::from(U256::ONE));
    assert_eq!(proxy.count("eth_getBlockAccessList"), 1);
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
    assert_eq!(replacement.count("eth_getBlockAccessList"), 0);
    assert_eq!(replacement.count("eth_getStorageAt"), 1);
}
