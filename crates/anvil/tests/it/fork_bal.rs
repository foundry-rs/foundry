//! Fork cache warming from immutable block access lists.

use alloy_genesis::{Genesis, GenesisAccount};
use alloy_network::TransactionBuilder;
use alloy_primitives::{Address, B256, U256, address, bytes};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, TransactionRequest, anvil::Forking};
use alloy_serde::WithOtherFields;
use anvil::{
    EthereumHardfork, NodeConfig, NodeHandle,
    eth::{
        EthApi,
        backend::db::{SerializableAccountRecord, SerializableState},
    },
    spawn,
};
use axum::{Json, Router, routing::post};
use foundry_primitives::FoundryNetwork;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::task::JoinHandle;

mod accounts;
mod identity;
mod reset;

const CONTRACT: Address = address!("000000000000000000000000000000000000ba10");

#[derive(Clone, Copy, Debug)]
enum BalResponse {
    Valid,
    CanonicalOnly,
    LegacyOnly,
    Timing,
    Unsupported,
    InternalError,
    Null,
    BadCommitment,
    WithoutCommitment,
    PreCancun,
    Malformed,
    Timeout,
    LegacyTimeout,
    NodeInfoTimeout,
    NodeInfoTimeoutOnce,
    NodeInfoInternalError,
    NodeInfoInternalErrorAfterDiscovery,
    NodeInfoMalformed,
    MetadataTimeout,
}

struct BalProxy {
    endpoint: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: JoinHandle<()>,
}

impl BalProxy {
    async fn new(origin: &NodeHandle, mode: BalResponse, reveal_anvil: bool) -> Self {
        Self::with_latency(origin, mode, reveal_anvil, Duration::ZERO).await
    }

    async fn with_latency(
        origin: &NodeHandle,
        mode: BalResponse,
        reveal_anvil: bool,
        latency: Duration,
    ) -> Self {
        let upstream = origin.http_endpoint();
        let client = reqwest::Client::new();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let router = Router::new().route(
            "/",
            post(move |Json(request): Json<Value>| {
                let upstream = upstream.clone();
                let client = client.clone();
                let recorded = Arc::clone(&recorded);
                async move {
                    recorded.lock().push(request.clone());
                    if !latency.is_zero() {
                        tokio::time::sleep(latency).await;
                    }
                    let method = request["method"].as_str().unwrap();
                    if method == "anvil_nodeInfo" {
                        let calls =
                            recorded.lock().iter().filter(|r| r["method"] == method).count();
                        match mode {
                            BalResponse::NodeInfoTimeout => {
                                return futures::future::pending().await;
                            }
                            BalResponse::NodeInfoTimeoutOnce if calls == 1 => {
                                return futures::future::pending().await;
                            }
                            BalResponse::NodeInfoInternalError
                            | BalResponse::NodeInfoInternalErrorAfterDiscovery
                                if matches!(mode, BalResponse::NodeInfoInternalError)
                                    || calls > 2 =>
                            {
                                return Json(json!({
                                    "jsonrpc": "2.0", "id": request["id"],
                                    "error": {"code": -32603, "message": "injected internal error"},
                                }));
                            }
                            BalResponse::NodeInfoMalformed => {
                                return Json(json!({
                                    "jsonrpc": "2.0", "id": request["id"], "result": {},
                                }));
                            }
                            _ => {}
                        }
                    }
                    if method == "anvil_metadata" && matches!(mode, BalResponse::MetadataTimeout) {
                        return futures::future::pending().await;
                    }
                    if (!reveal_anvil && matches!(method, "anvil_nodeInfo" | "anvil_metadata"))
                        || (method == "eth_getAccountInfo" && matches!(mode, BalResponse::Timing))
                        || (matches!(
                            method,
                            "eth_getBlockAccessList" | "eth_getBlockAccessListByBlockHash"
                        ) && matches!(mode, BalResponse::Unsupported))
                        || (method == "eth_getBlockAccessListByBlockHash"
                            && matches!(mode, BalResponse::CanonicalOnly))
                        || (method == "eth_getBlockAccessList"
                            && matches!(mode, BalResponse::LegacyOnly | BalResponse::LegacyTimeout))
                    {
                        return Json(json!({
                            "jsonrpc": "2.0", "id": request["id"],
                            "error": {"code": -32601, "message": "method not found"},
                        }));
                    }
                    if method == "eth_getBlockAccessListByBlockHash"
                        && matches!(mode, BalResponse::LegacyTimeout)
                    {
                        return futures::future::pending().await;
                    }
                    if method == "eth_getBlockAccessList" {
                        match mode {
                            BalResponse::InternalError => {
                                return Json(json!({
                                    "jsonrpc": "2.0", "id": request["id"],
                                    "error": {"code": -32603, "message": "injected internal error"},
                                }));
                            }
                            BalResponse::Null => {
                                return Json(json!({
                                    "jsonrpc": "2.0", "id": request["id"], "result": null,
                                }));
                            }
                            BalResponse::Malformed => {
                                return Json(json!({
                                    "jsonrpc": "2.0", "id": request["id"],
                                    "result": [{"address": "invalid"}],
                                }));
                            }
                            BalResponse::Timeout => return futures::future::pending().await,
                            _ => {}
                        }
                    }
                    let mut response = client
                        .post(upstream)
                        .json(&request)
                        .send()
                        .await
                        .unwrap()
                        .json::<Value>()
                        .await
                        .unwrap();
                    if method == "eth_getBlockAccessList"
                        && matches!(mode, BalResponse::BadCommitment)
                    {
                        response["result"] = json!([]);
                    }
                    if matches!(method, "eth_getBlockByHash" | "eth_getBlockByNumber")
                        && let Some(block) = response["result"].as_object_mut()
                    {
                        if matches!(mode, BalResponse::WithoutCommitment | BalResponse::PreCancun) {
                            block.remove("blockAccessListHash");
                        }
                        if matches!(mode, BalResponse::PreCancun) {
                            block.insert("timestamp".to_owned(), json!("0x0"));
                        }
                    }
                    Json(response)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self { endpoint, requests, task }
    }

    fn count(&self, method: &str) -> usize {
        self.requests.lock().iter().filter(|request| request["method"] == method).count()
    }

    fn clear(&self) {
        self.requests.lock().clear();
    }
}

impl Drop for BalProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct BalOrigin {
    api: EthApi<FoundryNetwork>,
    handle: NodeHandle,
    sender: Address,
    block_number: u64,
    block_hash: B256,
}

impl BalOrigin {
    async fn new() -> Self {
        Self::with_genesis_block_number(0).await
    }

    async fn with_genesis_block_number(number: u64) -> Self {
        let (api, handle) = spawn(
            NodeConfig::test()
                .with_chain_id(Some(1u64))
                .with_genesis_block_number(Some(number))
                .with_hardfork(Some(EthereumHardfork::Amsterdam.into()))
                .with_genesis_timestamp(Some(1_800_000_000u64))
                .with_no_mining(true),
        )
        .await;
        let sender = handle.dev_wallets().next().unwrap().address();
        // Read slot one without changing it, then increment slot zero.
        api.anvil_set_code(CONTRACT, bytes!("6001545060005460010160005500")).await.unwrap();
        api.anvil_set_storage_at(CONTRACT, U256::from(1), B256::from(U256::from(9))).await.unwrap();
        Self::increment(&api, sender).await;
        let block = handle
            .http_provider()
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await
            .unwrap()
            .unwrap();
        assert!(block.header.block_access_list_hash.is_some());
        Self {
            api,
            handle,
            sender,
            block_number: block.header.number,
            block_hash: block.header.hash,
        }
    }

    fn config(&self, proxy: &BalProxy) -> NodeConfig {
        NodeConfig::test()
            .with_eth_rpc_url(Some(proxy.endpoint.clone()))
            .with_fork_block_number(Some(self.block_number))
            .with_hardfork(Some(EthereumHardfork::Amsterdam.into()))
            .with_no_storage_caching(true)
            .with_genesis_accounts(vec![])
            .with_no_mining(true)
    }

    async fn increment(api: &EthApi<FoundryNetwork>, sender: Address) {
        api.send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .with_from(sender)
                .with_to(CONTRACT)
                .with_gas_limit(200_000),
        ))
        .await
        .unwrap();
        api.mine_one().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_seeds_changed_slots_without_eager_account_requests() {
    let origin = BalOrigin::new().await;
    let proxy = BalProxy::new(&origin.handle, BalResponse::CanonicalOnly, false).await;
    let (api, _handle) = spawn(origin.config(&proxy)).await;

    assert_eq!(proxy.count("eth_getBlockAccessList"), 1);
    assert_eq!(proxy.count("eth_getBlockAccessListByBlockHash"), 0);
    for method in ["eth_getAccountInfo", "eth_getBalance", "eth_getTransactionCount", "eth_getCode"]
    {
        assert_eq!(proxy.count(method), 0, "{method} must remain lazy");
    }
    let bal_request = proxy
        .requests
        .lock()
        .iter()
        .find(|request| request["method"] == "eth_getBlockAccessList")
        .cloned()
        .unwrap();
    assert_eq!(bal_request["params"][0], json!(origin.block_hash));

    proxy.clear();
    assert_eq!(api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(), B256::from(U256::ONE));
    assert_eq!(proxy.count("eth_getStorageAt"), 0);
    assert_eq!(
        api.storage_at(CONTRACT, U256::from(1), None).await.unwrap(),
        B256::from(U256::from(9))
    );
    assert_eq!(proxy.count("eth_getStorageAt"), 1, "read-only BAL slots require RPC fallback");
    assert_eq!(
        api.balance(origin.sender, None).await.unwrap(),
        origin.api.balance(origin.sender, None).await.unwrap()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_falls_back_to_legacy_method_when_canonical_is_unsupported() {
    let origin = BalOrigin::new().await;
    let proxy = BalProxy::new(&origin.handle, BalResponse::LegacyOnly, false).await;
    let (api, _handle) = spawn(origin.config(&proxy)).await;
    let requests = proxy
        .requests
        .lock()
        .iter()
        .filter(|request| {
            matches!(
                request["method"].as_str(),
                Some("eth_getBlockAccessList" | "eth_getBlockAccessListByBlockHash")
            )
        })
        .map(|request| (request["method"].clone(), request["params"].clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        requests,
        vec![
            (json!("eth_getBlockAccessList"), json!([origin.block_hash])),
            (json!("eth_getBlockAccessListByBlockHash"), json!([origin.block_hash])),
        ]
    );
    proxy.clear();
    assert_eq!(api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(), B256::from(U256::ONE));
    assert_eq!(proxy.count("eth_getStorageAt"), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_matches_lazy_state_across_commits_snapshots_and_reset() {
    let origin = BalOrigin::new().await;
    let mut final_balances = Vec::new();
    for no_bal in [false, true] {
        let proxy = BalProxy::new(&origin.handle, BalResponse::Valid, false).await;
        let (api, _handle) = spawn(origin.config(&proxy).with_no_bal(no_bal)).await;
        assert_eq!(proxy.count("eth_getBlockAccessList"), usize::from(!no_bal));
        let snapshot = api.evm_snapshot().await.unwrap();
        for value in [2, 3] {
            BalOrigin::increment(&api, origin.sender).await;
            assert_eq!(
                api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
                B256::from(U256::from(value)),
            );
        }
        final_balances.push(api.balance(origin.sender, None).await.unwrap());
        assert!(api.evm_revert(snapshot).await.unwrap());
        assert_eq!(
            api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
            B256::from(U256::ONE)
        );
        let snapshot = api.evm_snapshot().await.unwrap();
        api.anvil_set_storage_at(CONTRACT, U256::ZERO, B256::from(U256::from(99))).await.unwrap();
        api.anvil_reset(Some(Forking {
            json_rpc_url: None,
            block_number: Some(origin.block_number),
        }))
        .await
        .unwrap();
        assert!(!api.evm_revert(snapshot).await.unwrap());
        assert_eq!(
            api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
            B256::from(U256::ONE)
        );
        assert_eq!(proxy.count("eth_getBlockAccessList"), 2 * usize::from(!no_bal));
    }
    assert_eq!(final_balances[0], final_balances[1]);
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_failure_falls_back_without_delaying_normal_rpc_timeout() {
    let origin = BalOrigin::new().await;
    for mode in [
        BalResponse::Unsupported,
        BalResponse::InternalError,
        BalResponse::Null,
        BalResponse::BadCommitment,
        BalResponse::Malformed,
        BalResponse::Timeout,
        BalResponse::LegacyTimeout,
    ] {
        let proxy = BalProxy::new(&origin.handle, mode, false).await;
        let (api, _handle) = tokio::time::timeout(
            Duration::from_secs(5),
            spawn(origin.config(&proxy).fork_request_timeout(Some(Duration::from_secs(60)))),
        )
        .await
        .unwrap_or_else(|_| panic!("optional BAL request must have a bounded deadline: {mode:?}"));
        assert_eq!(proxy.count("eth_getBlockAccessList"), 1, "{mode:?}");
        assert_eq!(
            proxy.count("eth_getBlockAccessListByBlockHash"),
            usize::from(matches!(mode, BalResponse::Unsupported | BalResponse::LegacyTimeout)),
            "{mode:?}"
        );
        proxy.clear();
        assert_eq!(
            api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
            B256::from(U256::ONE)
        );
        assert_eq!(proxy.count("eth_getStorageAt"), 1, "{mode:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_preserves_genesis_funding_and_loaded_state_overrides() {
    let origin = BalOrigin::new().await;
    let proxy = BalProxy::new(&origin.handle, BalResponse::Valid, false).await;
    let genesis = Genesis {
        alloc: [(
            CONTRACT,
            GenesisAccount {
                balance: U256::from(10),
                storage: Some([(B256::ZERO, B256::from(U256::from(20)))].into()),
                ..Default::default()
            },
        )]
        .into(),
        ..Default::default()
    };
    let config = origin
        .config(&proxy)
        .with_genesis(Some(genesis))
        .with_funded_accounts([(CONTRACT, U256::from(30))].into_iter().collect());
    let (api, _handle) = spawn(config.clone()).await;
    assert_eq!(
        api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
        B256::from(U256::from(20))
    );
    assert_eq!(api.balance(CONTRACT, None).await.unwrap(), U256::from(30));

    let state = SerializableState {
        accounts: [(
            CONTRACT,
            SerializableAccountRecord {
                nonce: 3,
                balance: U256::from(40),
                code: bytes!("00"),
                storage: [(B256::ZERO, B256::from(U256::from(50)))].into(),
            },
        )]
        .into(),
        ..Default::default()
    };
    let (api, _handle) = spawn(config.with_init_state(Some(state))).await;
    assert_eq!(
        api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
        B256::from(U256::from(50))
    );
    assert_eq!(api.balance(CONTRACT, None).await.unwrap(), U256::from(40));
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_skips_mutable_anvil_sources_including_historical_blocks() {
    let origin = BalOrigin::new().await;
    origin
        .api
        .anvil_set_storage_at(CONTRACT, U256::ZERO, B256::from(U256::from(99)))
        .await
        .unwrap();
    for historical in [false, true] {
        if historical {
            origin.api.mine_one().await.unwrap();
        }
        let proxy = BalProxy::new(&origin.handle, BalResponse::Valid, true).await;
        let (api, _handle) = spawn(origin.config(&proxy)).await;
        assert_eq!(proxy.count("eth_getBlockAccessList"), 0);
        assert_eq!(
            api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
            B256::from(U256::from(99)),
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_reset_replaces_the_remote_seed() {
    let origin = BalOrigin::new().await;
    let proxy = BalProxy::new(&origin.handle, BalResponse::Valid, false).await;
    let (api, _handle) = spawn(origin.config(&proxy)).await;
    assert_eq!(api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(), B256::from(U256::ONE));

    let replacement = BalOrigin::new().await;
    BalOrigin::increment(&replacement.api, replacement.sender).await;
    let replacement_proxy = BalProxy::new(&replacement.handle, BalResponse::Valid, false).await;
    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(replacement_proxy.endpoint.clone()),
        block_number: Some(replacement.block_number + 1),
    }))
    .await
    .unwrap();
    replacement_proxy.clear();
    assert_eq!(
        api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
        B256::from(U256::from(2))
    );
    assert_eq!(replacement_proxy.count("eth_getStorageAt"), 0);
    api.anvil_reset(None).await.unwrap();
    assert_eq!(api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(), B256::ZERO);
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_transaction_hash_uses_parent_seed_and_replays_target_prefix() {
    let origin = BalOrigin::new().await;
    let provider = origin.handle.http_provider();
    let nonce = provider.get_transaction_count(origin.sender).await.unwrap();
    let mut transactions = Vec::new();
    for nonce in [nonce, nonce + 1] {
        transactions.push(
            origin
                .api
                .send_transaction(WithOtherFields::new(
                    TransactionRequest::default()
                        .with_from(origin.sender)
                        .with_to(CONTRACT)
                        .with_gas_limit(200_000)
                        .with_nonce(nonce),
                ))
                .await
                .unwrap(),
        );
    }
    origin.api.mine_one().await.unwrap();
    assert_eq!(
        origin.api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
        B256::from(U256::from(3)),
    );

    let proxy = BalProxy::new(&origin.handle, BalResponse::Valid, false).await;
    let (api, _handle) =
        spawn(origin.config(&proxy).with_fork_transaction_hash(Some(transactions[0]))).await;
    let requested_blocks = proxy
        .requests
        .lock()
        .iter()
        .filter(|request| request["method"] == "eth_getBlockAccessList")
        .map(|request| request["params"][0].clone())
        .collect::<Vec<_>>();
    assert_eq!(requested_blocks, vec![json!(origin.block_hash)]);
    assert_eq!(
        api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
        B256::from(U256::from(2)),
    );
    assert!(api.backend.mined_transaction_by_hash(transactions[0]).is_some());
    assert!(api.backend.mined_transaction_by_hash(transactions[1]).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_accepts_uncommitted_history_but_skips_pre_cancun_sources() {
    let origin = BalOrigin::new().await;
    for (mode, expected_bal_calls, expected_storage_calls) in
        [(BalResponse::WithoutCommitment, 1, 0), (BalResponse::PreCancun, 0, 1)]
    {
        let proxy = BalProxy::new(&origin.handle, mode, false).await;
        // The explicit Amsterdam execution override must not change source eligibility.
        let (api, _handle) = spawn(origin.config(&proxy)).await;
        assert_eq!(proxy.count("eth_getBlockAccessList"), expected_bal_calls);
        assert_eq!(
            api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
            B256::from(U256::ONE),
        );
        assert_eq!(proxy.count("eth_getStorageAt"), expected_storage_calls);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_failed_reset_keeps_live_changes_and_snapshots() {
    let origin = BalOrigin::new().await;
    let proxy = BalProxy::new(&origin.handle, BalResponse::Valid, false).await;
    let (api, _handle) = spawn(origin.config(&proxy)).await;
    let snapshot = api.evm_snapshot().await.unwrap();
    api.anvil_set_storage_at(CONTRACT, U256::ZERO, B256::from(U256::from(77))).await.unwrap();
    let result = api
        .anvil_reset(Some(Forking {
            json_rpc_url: None,
            block_number: Some(origin.block_number + 100),
        }))
        .await;
    assert!(result.is_err());
    assert_eq!(api.block_number().unwrap(), U256::from(origin.block_number));
    assert_eq!(
        api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
        B256::from(U256::from(77)),
    );
    assert!(api.evm_revert(snapshot).await.unwrap());
    assert_eq!(api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(), B256::from(U256::ONE));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual local BAL timing experiment"]
async fn fork_bal_local_timing() {
    let origin = BalOrigin::new().await;
    let timestamp = origin
        .handle
        .http_provider()
        .get_block_by_hash(origin.block_hash)
        .await
        .unwrap()
        .unwrap()
        .header
        .timestamp;
    let mut samples = Vec::new();
    for delay_ms in [0, 20, 80] {
        for repeat in 0..5 {
            for no_bal in [false, true] {
                let proxy = BalProxy::with_latency(
                    &origin.handle,
                    BalResponse::Timing,
                    false,
                    Duration::from_millis(delay_ms),
                )
                .await;
                let start = Instant::now();
                let (api, _handle) = spawn(origin.config(&proxy).with_no_bal(no_bal)).await;
                let startup = start.elapsed();
                api.evm_set_next_block_timestamp(timestamp + 1).unwrap();
                BalOrigin::increment(&api, origin.sender).await;
                let first_transaction = start.elapsed();
                for offset in 2..=3 {
                    api.evm_set_next_block_timestamp(timestamp + offset).unwrap();
                    BalOrigin::increment(&api, origin.sender).await;
                }
                let total = start.elapsed();
                assert_eq!(
                    api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
                    B256::from(U256::from(4)),
                );
                samples.push(json!({
                    "delay_ms": delay_ms,
                    "no_bal": no_bal,
                    "repeat": repeat,
                    "startup_ms": startup.as_secs_f64() * 1000.0,
                    "startup_and_first_transaction_ms": first_transaction.as_secs_f64() * 1000.0,
                    "startup_and_three_transactions_ms": total.as_secs_f64() * 1000.0,
                    "bal_requests": proxy.count("eth_getBlockAccessList"),
                    "storage_requests": proxy.count("eth_getStorageAt"),
                    "account_requests": proxy.count("eth_getAccountInfo"),
                }));
            }
        }
    }
    foundry_common::sh_println!("{}", serde_json::to_string(&samples).unwrap()).unwrap();
}
