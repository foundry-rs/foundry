//! Failed fork resets preserve the live and persisted state after applying a candidate BAL.

use super::{BalOrigin, BalProxy, BalResponse, CONTRACT};
use alloy_primitives::{B256, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, anvil::Forking};
use anvil::spawn;
use axum::{Json, Router, routing::post};
use foundry_evm::backend::{BlockchainDb, BlockchainDbMeta};
use parking_lot::Mutex;
use revm::context::BlockEnv;
use serde_json::{Value, json};
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::task::JoinHandle;
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
    instrument::WithSubscriber,
};
use tracing_subscriber::{
    Layer,
    layer::{Context, SubscriberExt},
};

#[derive(Default)]
struct ResetFailure {
    block_number: Option<u64>,
    bal_returned: bool,
    rejected_blocks: usize,
}

/// Rejects the candidate block lookup that follows BAL application during reset.
struct ResetProxy {
    endpoint: String,
    failure: Arc<Mutex<ResetFailure>>,
    task: JoinHandle<()>,
}

impl ResetProxy {
    async fn new(proxy: &BalProxy) -> Self {
        let upstream = proxy.endpoint.clone();
        let client = reqwest::Client::new();
        let failure = Arc::new(Mutex::new(ResetFailure::default()));
        let control = Arc::clone(&failure);
        let router = Router::new().route(
            "/{namespace}",
            post(move |Json(request): Json<Value>| {
                let upstream = upstream.clone();
                let client = client.clone();
                let control = Arc::clone(&control);
                async move {
                    let method = request["method"].as_str().unwrap();
                    {
                        let mut failure = control.lock();
                        if method == "eth_getBlockByNumber"
                            && failure.bal_returned
                            && let Some(number) = failure.block_number
                            && request["params"][0] == json!(format!("{number:#x}"))
                        {
                            failure.rejected_blocks += 1;
                            return Json(json!({
                                "jsonrpc": "2.0", "id": request["id"], "result": null,
                            }));
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
                    if method == "eth_getBlockAccessListByBlockHash"
                        && control.lock().block_number.is_some()
                    {
                        assert!(response["result"].as_array().is_some_and(|bal| !bal.is_empty()));
                        control.lock().bal_returned = true;
                    }
                    Json(response)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        // A unique URL isolates persistent cache files without changing global cache settings.
        let endpoint = format!("http://{}/{}", listener.local_addr().unwrap(), B256::random());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self { endpoint, failure, task }
    }
}

impl Drop for ResetProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug, Default)]
struct SeedEvent {
    message: String,
    block_hash: String,
    inserted_slots: u64,
}

impl Visit for SeedEvent {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        match field.name() {
            "message" => self.message = format!("{value:?}"),
            "block_hash" => self.block_hash = format!("{value:?}"),
            _ => {}
        }
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "inserted_slots" {
            self.inserted_slots = value;
        }
    }
}

#[derive(Clone, Default)]
struct AppliedSeeds(Arc<Mutex<Vec<SeedEvent>>>);

impl<S: Subscriber> Layer<S> for AppliedSeeds {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if event.metadata().target() == "node" {
            let mut seed = SeedEvent::default();
            event.record(&mut seed);
            if seed.message == "prefilled fork cache from BAL" {
                self.0.lock().push(seed);
            }
        }
    }
}

struct CacheFiles([PathBuf; 2]);

impl Drop for CacheFiles {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn persisted_cache(meta: &BlockchainDbMeta<BlockEnv>, path: &Path) -> Value {
    assert!(path.is_file(), "persistent fork cache must exist");
    let db = BlockchainDb::new(meta.clone(), Some(path.to_path_buf()));
    json!({
        "meta": *db.meta().read(),
        "accounts": *db.accounts().read(),
        "storage": *db.storage().read(),
        "block_hashes": *db.block_hashes().read(),
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_failed_reset_after_seed_preserves_persistent_cache() {
    let origin =
        BalOrigin::with_genesis_block_number(50_000_000 + u64::from(rand::random::<u32>())).await;
    let proxy = BalProxy::new(&origin.handle, BalResponse::Valid, false).await;
    let reset_proxy = ResetProxy::new(&proxy).await;
    let config = origin
        .config(&proxy)
        .with_eth_rpc_url(Some(reset_proxy.endpoint.clone()))
        .with_chain_id(Some(1u64))
        .with_no_storage_caching(false);
    let old_path = config.block_cache_path(origin.block_number).unwrap();
    let candidate_path = config.block_cache_path(origin.block_number + 1).unwrap();
    assert!(!old_path.exists());
    assert!(!candidate_path.exists());
    let _cache_files = CacheFiles([old_path.clone(), candidate_path.clone()]);
    let (api, handle) = spawn(config.clone()).await;

    assert_eq!(api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(), B256::from(U256::ONE));
    // Include lazily fetched account data and a read-only BAL slot in the persisted baseline.
    api.balance(CONTRACT, None).await.unwrap();
    assert_eq!(api.storage_at(CONTRACT, U256::ONE, None).await.unwrap(), B256::from(U256::from(9)));
    let meta = {
        let db = api.backend.get_db().read().await;
        db.maybe_flush_cache().unwrap();
        let remote = db.maybe_inner().unwrap();
        assert_eq!(remote.cache().cache_path(), Some(old_path.as_path()));
        assert_eq!(remote.storage().read()[&CONTRACT][&U256::ZERO], U256::ONE);
        remote.meta().read().clone()
    };
    let baseline = persisted_cache(&meta, &old_path);
    let snapshot = api.evm_snapshot().await.unwrap();
    api.anvil_set_storage_at(CONTRACT, U256::ZERO, B256::from(U256::from(77))).await.unwrap();

    BalOrigin::increment(&origin.api, origin.sender).await;
    let candidate = origin
        .handle
        .http_provider()
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(candidate.header.number, origin.block_number + 1);
    assert_eq!(
        origin.api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
        B256::from(U256::from(2))
    );
    reset_proxy.failure.lock().block_number = Some(candidate.header.number);
    proxy.clear();
    let applied_seeds = AppliedSeeds::default();
    let result = api
        .anvil_reset(Some(Forking {
            json_rpc_url: None,
            block_number: Some(candidate.header.number),
        }))
        .with_subscriber(tracing_subscriber::registry().with(applied_seeds.clone()))
        .await;
    assert!(result.is_err());
    assert_eq!(proxy.count("eth_getBlockAccessListByBlockHash"), 1);
    assert_eq!(reset_proxy.failure.lock().rejected_blocks, 1);
    assert!(
        applied_seeds.0.lock().iter().any(|seed| {
            seed.block_hash == candidate.header.hash.to_string() && seed.inserted_slots > 0
        }),
        "reset must fail after the candidate BAL has populated its cache"
    );
    assert_eq!(api.block_number().unwrap(), U256::from(origin.block_number));
    assert_eq!(
        api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(),
        B256::from(U256::from(77)),
    );
    // Compare decoded contents: map serialization can change key order without changing state.
    assert_eq!(persisted_cache(&meta, &old_path), baseline);
    assert!(!candidate_path.exists(), "a rejected candidate must not persist its seeded cache");
    assert!(api.evm_revert(snapshot).await.unwrap());
    assert_eq!(api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(), B256::from(U256::ONE));

    drop(handle);
    drop(api);
    *reset_proxy.failure.lock() = ResetFailure::default();
    proxy.clear();
    let (api, _handle) = spawn(config.with_no_bal(true)).await;
    assert_eq!(api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(), B256::from(U256::ONE));
    assert_eq!(api.storage_at(CONTRACT, U256::ONE, None).await.unwrap(), B256::from(U256::from(9)));
    assert_eq!(proxy.count("eth_getBlockAccessListByBlockHash"), 0);
    assert_eq!(proxy.count("eth_getStorageAt"), 0, "the reopened fork must read its disk cache");
    assert!(!candidate_path.exists());
}
