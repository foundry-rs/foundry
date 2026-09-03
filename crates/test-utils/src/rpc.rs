//! RPC testing utilities.

use alloy_primitives::B256;
use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::post};
use foundry_config::{
    NamedChain::{
        self, Arbitrum, Base, BinanceSmartChainTestnet, Celo, Gnosis, Hyperliquid, Mainnet,
        Optimism, Polygon, Robinhood, Sepolia,
    },
    RpcEndpointUrl, RpcEndpoints,
};
use rand::seq::SliceRandom;
use serde_json::{Value, json};
use std::{
    env,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

macro_rules! shuffled_list {
    ($name:ident, $e:expr $(,)?) => {
        static $name: LazyLock<ShuffledList<&'static str>> =
            LazyLock::new(|| ShuffledList::new($e));
    };
}

struct ShuffledList<T> {
    list: Vec<T>,
    index: AtomicUsize,
}

impl<T> ShuffledList<T> {
    fn new(mut list: Vec<T>) -> Self {
        assert!(!list.is_empty());
        list.shuffle(&mut rand::rng());
        Self { list, index: AtomicUsize::new(0) }
    }

    fn next(&self) -> &T {
        let index = self.index.fetch_add(1, Ordering::Relaxed);
        &self.list[index % self.list.len()]
    }
}

shuffled_list!(
    HTTP_ARCHIVE_DOMAINS,
    vec![
        //
        "ethereum.reth.rs/rpc",
    ],
);
shuffled_list!(
    HTTP_DOMAINS,
    vec![
        //
        "ethereum.reth.rs/rpc",
    ],
);
shuffled_list!(
    WS_ARCHIVE_DOMAINS,
    vec![
        //
        "ethereum.reth.rs/ws",
    ],
);
shuffled_list!(
    WS_DOMAINS,
    vec![
        //
        "ethereum.reth.rs/ws",
    ],
);

// Public Arbitrum endpoints, rotated so that a retry reaches a different provider.
//
// Every entry must serve archive state: `fork::flaky_test_arb_fork_mining` forks at a pinned block
// far behind the head, which non-archive endpoints such as `arb1.arbitrum.io` reject with
// `missing trie node`. The DRPC keys used for the other chains do not qualify: their Arbitrum quota
// is exhausted and every fork of it fails.
shuffled_list!(
    ARBITRUM_URLS,
    vec![
        //
        "https://arb-pokt.nodies.app",
        "https://arbitrum.gateway.tenderly.co",
    ],
);

// List of general purpose DRPC keys to rotate through
shuffled_list!(
    DRPC_KEYS,
    vec![
        "Agc9NK9-6UzYh-vQDDM80Tv0A5UnBkUR8I3qssvAG40d",
        "AjUPUPonSEInt2CZ_7A-ai3hMyxxBlsR8I4EssvAG40d",
    ],
);

// List of etherscan keys.
shuffled_list!(
    ETHERSCAN_KEYS,
    vec![
        "MCAUM7WPE9XP5UQMZPCKIBUJHPM1C24FP6",
        "JW6RWCG2C5QF8TANH4KC7AYIF1CX7RB5D1",
        "ZSMDY6BI2H55MBE3G9CUUQT4XYUDBB6ZSK",
        "4FYHTY429IXYMJNS4TITKDMUKW5QRYDX61",
        "QYKNT5RHASZ7PGQE68FNQWH99IXVTVVD2I",
        "VXMQ117UN58Y4RHWUB8K1UGCEA7UQEWK55",
        "C7I2G4JTA5EPYS42Z8IZFEIMQNI5GXIJEV",
        "A15KZUMZXXCK1P25Y1VP1WGIVBBHIZDS74",
        "3IA6ASNQXN8WKN7PNFX7T72S9YG56X9FPG",
    ],
);

/// the RPC endpoints used during tests
pub fn rpc_endpoints() -> RpcEndpoints {
    RpcEndpoints::new([
        ("mainnet", RpcEndpointUrl::Url(next_http_archive_rpc_url())),
        ("mainnet2", RpcEndpointUrl::Url(next_http_archive_rpc_url())),
        ("sepolia", RpcEndpointUrl::Url(next_rpc_endpoint(NamedChain::Sepolia))),
        ("optimism", RpcEndpointUrl::Url(next_rpc_endpoint(NamedChain::Optimism))),
        ("base", RpcEndpointUrl::Url(next_rpc_endpoint(NamedChain::Base))),
        ("arbitrum", RpcEndpointUrl::Url(next_rpc_endpoint(NamedChain::Arbitrum))),
        ("polygon", RpcEndpointUrl::Url(next_rpc_endpoint(NamedChain::Polygon))),
        ("bsc", RpcEndpointUrl::Url(next_rpc_endpoint(NamedChain::BinanceSmartChain))),
        ("avaxTestnet", RpcEndpointUrl::Url("https://api.avax-test.network/ext/bc/C/rpc".into())),
        ("moonbeam", RpcEndpointUrl::Url("https://moonbeam.api.onfinality.io/public".into())),
        ("polkadotTestnet", RpcEndpointUrl::Url("https://eth-rpc-testnet.polkadot.io".into())),
        ("kusama", RpcEndpointUrl::Url("https://eth-rpc-kusama.polkadot.io".into())),
        ("polkadot", RpcEndpointUrl::Url("https://eth-rpc.polkadot.io".into())),
        ("rpcEnvAlias", RpcEndpointUrl::Env("${RPC_ENV_ALIAS}".into())),
    ])
}

/// Returns the next _mainnet_ rpc URL in inline
///
/// This will rotate all available rpc endpoints
pub fn next_http_rpc_endpoint() -> String {
    next_rpc_endpoint(NamedChain::Mainnet)
}

/// Returns the next _mainnet_ rpc URL in inline
///
/// This will rotate all available rpc endpoints
pub fn next_ws_rpc_endpoint() -> String {
    next_ws_endpoint(NamedChain::Mainnet)
}

/// Returns the next HTTP RPC URL.
pub fn next_rpc_endpoint(chain: NamedChain) -> String {
    next_url(false, chain)
}

/// Returns the next WS RPC URL.
pub fn next_ws_endpoint(chain: NamedChain) -> String {
    next_url(true, chain)
}

/// Returns an HTTP URL that has access to archive state
pub fn next_http_archive_rpc_url() -> String {
    next_archive_url(false)
}

/// Returns a websocket URL that has access to archive state
pub fn next_ws_archive_rpc_url() -> String {
    next_archive_url(true)
}

/// Returns a URL that has access to archive state.
fn next_archive_url(is_ws: bool) -> String {
    let domain = if is_ws { &WS_ARCHIVE_DOMAINS } else { &HTTP_ARCHIVE_DOMAINS }.next();
    let url = if is_ws { format!("wss://{domain}") } else { format!("https://{domain}") };
    test_debug!("next_archive_url(is_ws={is_ws}) = {}", debug_url(&url));
    url
}

/// Returns the next etherscan api key.
pub fn next_etherscan_api_key() -> String {
    let mut key = env::var("ETHERSCAN_KEY").unwrap_or_default();
    if key.is_empty() {
        key = ETHERSCAN_KEYS.next().to_string();
    }
    test_debug!("next_etherscan_api_key() = {}...", &key[..6]);
    key
}

fn next_url(is_ws: bool, chain: NamedChain) -> String {
    let url = next_url_inner(is_ws, chain);
    test_debug!("next_url(is_ws={is_ws}, chain={chain:?}) = {}", debug_url(&url));
    url
}

fn next_url_inner(is_ws: bool, chain: NamedChain) -> String {
    if matches!(chain, Base) {
        return "https://mainnet.base.org".to_string();
    }

    if matches!(chain, Optimism) {
        return "https://mainnet.optimism.io".to_string();
    }

    if matches!(chain, BinanceSmartChainTestnet) {
        return "https://bsc-testnet.bnbchain.org".to_string();
    }

    if matches!(chain, Celo) {
        // Not `celo.drpc.org`: it load balances across upstreams that disagree on the chain head,
        // so a fork of it regularly fails to fetch the block it just resolved.
        return env_rpc_url("CELO_RPC").unwrap_or_else(|| "https://forno.celo.org".to_string());
    }

    if matches!(chain, Gnosis) {
        return env_rpc_url("GNOSIS_RPC")
            .unwrap_or_else(|| "https://rpc.gnosischain.com".to_string());
    }

    if matches!(chain, Hyperliquid) {
        return env_rpc_url("HYPERLIQUID_RPC")
            .unwrap_or_else(|| "https://rpc.hyperliquid.xyz/evm".to_string());
    }

    if matches!(chain, Robinhood) {
        return env_rpc_url("ROBINHOOD_RPC")
            .unwrap_or_else(|| "https://rpc.mainnet.chain.robinhood.com".to_string());
    }

    if matches!(chain, Sepolia) {
        if let Some(rpc_url) = env_rpc_url("ETH_SEPOLIA_RPC") {
            return rpc_url;
        }
        return "https://ethereum-sepolia-rpc.publicnode.com".to_string();
    }

    if matches!(chain, Arbitrum) {
        return env_rpc_url("ARBITRUM_RPC").unwrap_or_else(|| (*ARBITRUM_URLS.next()).to_string());
    }

    let reth_works = true;
    let domain = if reth_works && matches!(chain, Mainnet) {
        *(if is_ws { &WS_DOMAINS } else { &HTTP_DOMAINS }).next()
    } else {
        // DRPC for other networks used in tests.
        let key = DRPC_KEYS.next();
        let network = match chain {
            Mainnet => "ethereum",
            Polygon => "polygon",
            Sepolia => "sepolia",
            _ => "",
        };
        &format!("lb.drpc.org/ogrpc?network={network}&dkey={key}")
    };

    if is_ws { format!("wss://{domain}") } else { format!("https://{domain}") }
}

/// Returns the RPC URL configured in the `var` environment variable, if it is set and non-empty.
fn env_rpc_url(var: &str) -> Option<String> {
    env::var(var).ok().filter(|url| !url.is_empty())
}

/// Basic redaction for debugging RPC URLs.
fn debug_url(url: &str) -> impl std::fmt::Display + '_ {
    let url = reqwest::Url::parse(url).unwrap();
    format!(
        "{scheme}://{host}{path}",
        scheme = url.scheme(),
        host = url.host_str().unwrap(),
        path = url.path().get(..8).unwrap_or(url.path()),
    )
}

const MONAD_SYSTEM_ADDRESS: &str = "0x6f49a8f621353f12378d0046e7d7e4b9b249dc9e";

/// Spawns an RPC proxy that presents one transaction as a canonical Monad protocol envelope.
pub async fn spawn_canonical_monad_system_rpc(endpoint: String, target_hash: B256) -> String {
    let target_hash = target_hash.to_string();
    let client = reqwest::Client::new();
    let router = Router::new().route(
        "/",
        post(move |Json(request): Json<Value>| {
            let client = client.clone();
            let endpoint = endpoint.clone();
            let target_hash = target_hash.clone();
            async move {
                let mut response = client
                    .post(endpoint)
                    .json(&request)
                    .send()
                    .await
                    .unwrap()
                    .json::<Value>()
                    .await
                    .unwrap();

                canonicalize_monad_system_response(&request, &mut response, &target_hash);

                Json(response)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{address}")
}

/// Spawns an RPC proxy that rejects `method` after forwarding `successful_calls` requests.
///
/// Rejections use an HTTP 403 response with a vendor-specific JSON-RPC error code. This models
/// gateways that deny unknown or custom methods without using the standard method-not-found code.
pub async fn spawn_rpc_proxy_rejecting_method_after(
    endpoint: String,
    method: &'static str,
    successful_calls: usize,
) -> String {
    spawn_rpc_proxy_rejecting_method(
        endpoint,
        method,
        RpcMethodRejection::After(successful_calls),
        StatusCode::FORBIDDEN,
        -32004,
        "method is not allowed",
    )
    .await
}

/// Spawns an RPC proxy whose rejection of `method` can be enabled after startup.
pub async fn spawn_rpc_proxy_rejecting_method_when_enabled(
    endpoint: String,
    method: &'static str,
) -> (String, Arc<AtomicBool>) {
    let enabled = Arc::new(AtomicBool::new(false));
    let proxy = spawn_rpc_proxy_rejecting_method(
        endpoint,
        method,
        RpcMethodRejection::Enabled(enabled.clone()),
        StatusCode::FORBIDDEN,
        -32004,
        "method is not allowed",
    )
    .await;
    (proxy, enabled)
}

/// Spawns an RPC proxy that returns method-not-found for the first `unavailable_calls` requests to
/// `method`.
pub async fn spawn_rpc_proxy_method_not_found_before(
    endpoint: String,
    method: &'static str,
    unavailable_calls: usize,
) -> String {
    spawn_rpc_proxy_rejecting_method(
        endpoint,
        method,
        RpcMethodRejection::Before(unavailable_calls),
        StatusCode::OK,
        -32601,
        "method not found",
    )
    .await
}

/// Spawns an RPC proxy that returns a JSON-RPC internal error for `method` after forwarding
/// `successful_calls` requests.
pub async fn spawn_rpc_proxy_internal_error_after(
    endpoint: String,
    method: &'static str,
    successful_calls: usize,
) -> String {
    spawn_rpc_proxy_rejecting_method(
        endpoint,
        method,
        RpcMethodRejection::After(successful_calls),
        StatusCode::OK,
        -32603,
        "internal error",
    )
    .await
}

/// Spawns an RPC proxy that answers `method` with `result` instead of forwarding it upstream.
///
/// All other methods are forwarded. The returned counter tracks how many `method` calls reached the
/// proxy, which lets tests assert that a request was never sent upstream.
pub async fn spawn_rpc_proxy_canned_method(
    endpoint: String,
    method: &'static str,
    result: Value,
) -> (String, Arc<AtomicUsize>) {
    let client = reqwest::Client::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let proxy_calls = calls.clone();
    let router = Router::new().route(
        "/",
        post(move |Json(request): Json<Value>| {
            let client = client.clone();
            let endpoint = endpoint.clone();
            let calls = proxy_calls.clone();
            let result = result.clone();
            async move {
                if request.get("method").and_then(Value::as_str) == Some(method) {
                    calls.fetch_add(1, Ordering::Relaxed);
                    let id = request.get("id").cloned().unwrap_or(Value::Null);
                    return Json(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result,
                    }));
                }

                let response = client
                    .post(endpoint)
                    .json(&request)
                    .send()
                    .await
                    .unwrap()
                    .json::<Value>()
                    .await
                    .unwrap();
                Json(response)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{address}"), calls)
}

/// Spawns an RPC proxy that reports the first transaction of every full block under `tx_type`.
///
/// Chains anvil can fork but not execute, such as Arbitrum and its Orbit rollups, open their
/// blocks with a system transaction of a type Foundry does not model. This reproduces that shape
/// on top of any endpoint, without depending on a public archive node.
pub async fn spawn_rpc_proxy_retyping_first_block_transaction(
    endpoint: String,
    tx_type: &'static str,
) -> String {
    let client = reqwest::Client::new();
    let router = Router::new().route(
        "/",
        post(move |Json(request): Json<Value>| {
            let client = client.clone();
            let endpoint = endpoint.clone();
            async move {
                let mut response = client
                    .post(endpoint)
                    .json(&request)
                    .send()
                    .await
                    .unwrap()
                    .json::<Value>()
                    .await
                    .unwrap();
                let responses = match response.as_array_mut() {
                    Some(batch) => batch.iter_mut().collect::<Vec<_>>(),
                    None => vec![&mut response],
                };
                for response in responses {
                    if let Some(transactions) = response
                        .get_mut("result")
                        .and_then(|result| result.get_mut("transactions"))
                        .and_then(Value::as_array_mut)
                        && let Some(first) = transactions.first_mut().and_then(Value::as_object_mut)
                    {
                        first.insert("type".to_string(), Value::from(tx_type));
                    }
                }
                Json(response).into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{address}")
}

#[derive(Clone)]
enum RpcMethodRejection {
    Before(usize),
    After(usize),
    Enabled(Arc<AtomicBool>),
}

impl RpcMethodRejection {
    fn rejects(&self, call: usize) -> bool {
        match self {
            Self::Before(rejected_calls) => call < *rejected_calls,
            Self::After(successful_calls) => call >= *successful_calls,
            Self::Enabled(enabled) => enabled.load(Ordering::SeqCst),
        }
    }
}

async fn spawn_rpc_proxy_rejecting_method(
    endpoint: String,
    method: &'static str,
    rejection: RpcMethodRejection,
    rejection_status: StatusCode,
    error_code: i64,
    error_message: &'static str,
) -> String {
    let client = reqwest::Client::new();
    let calls = std::sync::Arc::new(AtomicUsize::new(0));
    let router = Router::new().route(
        "/",
        post(move |Json(request): Json<Value>| {
            let client = client.clone();
            let endpoint = endpoint.clone();
            let calls = calls.clone();
            let rejection = rejection.clone();
            async move {
                if request.get("method").and_then(Value::as_str) == Some(method)
                    && rejection.rejects(calls.fetch_add(1, Ordering::Relaxed))
                {
                    let id = request.get("id").cloned().unwrap_or(Value::Null);
                    return (
                        rejection_status,
                        Json(json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": {
                                "code": error_code,
                                "message": error_message,
                            },
                        })),
                    )
                        .into_response();
                }

                let response = client
                    .post(endpoint)
                    .json(&request)
                    .send()
                    .await
                    .unwrap()
                    .json::<Value>()
                    .await
                    .unwrap();
                Json(response).into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{address}")
}

fn canonicalize_monad_system_response(request: &Value, response: &mut Value, target_hash: &str) {
    if let Some(requests) = request.as_array() {
        let Some(responses) = response.as_array_mut() else { return };
        for response in responses {
            let Some(response_id) = response.get("id") else { continue };
            if let Some(request) =
                requests.iter().find(|request| request.get("id") == Some(response_id))
            {
                canonicalize_monad_system_result(request, response, target_hash);
            }
        }
    } else {
        canonicalize_monad_system_result(request, response, target_hash);
    }
}

fn canonicalize_monad_system_result(request: &Value, response: &mut Value, target_hash: &str) {
    let Some(method) = request.get("method").and_then(Value::as_str) else { return };
    let Some(result) = response.get_mut("result") else { return };

    match method {
        "eth_getTransactionByHash"
        | "eth_getTransactionByBlockHashAndIndex"
        | "eth_getTransactionByBlockNumberAndIndex" => {
            canonicalize_monad_system_transaction(result, target_hash);
        }
        "eth_getBlockByHash" | "eth_getBlockByNumber" => {
            if let Some(transactions) = result.get_mut("transactions").and_then(Value::as_array_mut)
            {
                for transaction in transactions {
                    canonicalize_monad_system_transaction(transaction, target_hash);
                }
            }
        }
        "eth_getTransactionReceipt" => {
            canonicalize_monad_system_receipt(result, target_hash);
        }
        "eth_getBlockReceipts" => {
            if let Some(receipts) = result.as_array_mut() {
                for receipt in receipts {
                    canonicalize_monad_system_receipt(receipt, target_hash);
                }
            }
        }
        _ => {}
    }
}

fn canonicalize_monad_system_transaction(transaction: &mut Value, target_hash: &str) {
    let Some(transaction) = transaction.as_object_mut() else { return };
    if !transaction
        .get("hash")
        .and_then(Value::as_str)
        .is_some_and(|hash| hash.eq_ignore_ascii_case(target_hash))
    {
        return;
    }

    let tx_type = transaction.get("type").and_then(parse_rpc_quantity).unwrap_or_default();
    let legacy_v = (tx_type != 0)
        .then(|| {
            let parity = transaction
                .get("yParity")
                .or_else(|| transaction.get("v"))
                .and_then(parse_rpc_quantity)
                .filter(|parity| *parity <= 1)?;
            let v = if let Some(chain_id) = transaction.get("chainId").and_then(parse_rpc_quantity)
            {
                chain_id.checked_mul(2)?.checked_add(35 + parity)?
            } else {
                27 + parity
            };
            Some(format!("0x{v:x}"))
        })
        .flatten();

    transaction.insert("from".to_string(), json!(MONAD_SYSTEM_ADDRESS));
    transaction.insert("gas".to_string(), json!("0x0"));
    transaction.insert("gasPrice".to_string(), json!("0x0"));
    transaction.insert("type".to_string(), json!("0x0"));
    if let Some(v) = legacy_v {
        transaction.insert("v".to_string(), json!(v));
    }
    for field in [
        "accessList",
        "authorizationList",
        "blobVersionedHashes",
        "maxFeePerBlobGas",
        "maxFeePerGas",
        "maxPriorityFeePerGas",
        "yParity",
    ] {
        transaction.remove(field);
    }
}

fn parse_rpc_quantity(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value.as_str()?.strip_prefix("0x").and_then(|value| u64::from_str_radix(value, 16).ok())
    })
}

fn canonicalize_monad_system_receipt(receipt: &mut Value, target_hash: &str) {
    let Some(receipt) = receipt.as_object_mut() else { return };
    if !receipt
        .get("transactionHash")
        .and_then(Value::as_str)
        .is_some_and(|hash| hash.eq_ignore_ascii_case(target_hash))
    {
        return;
    }

    receipt.insert("cumulativeGasUsed".to_string(), json!("0x0"));
    receipt.insert("effectiveGasPrice".to_string(), json!("0x0"));
    receipt.insert("gasUsed".to_string(), json!("0x0"));
    receipt.insert("type".to_string(), json!("0x0"));
    receipt.remove("blobGasPrice");
    receipt.remove("blobGasUsed");
}

#[cfg(test)]
#[expect(clippy::disallowed_macros)]
mod tests {
    use super::*;
    use alloy_primitives::address;
    use foundry_config::Chain;

    #[test]
    fn canonical_monad_system_response_supports_batches() {
        let target_hash = B256::with_last_byte(1).to_string();
        let requests = json!([
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "eth_getTransactionByHash",
                "params": [target_hash],
            },
            {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "eth_getTransactionReceipt",
                "params": [target_hash],
            },
        ]);
        let mut responses = json!([
            {
                "jsonrpc": "2.0",
                "id": 2,
                "result": {
                    "transactionHash": target_hash,
                    "gasUsed": "0x5208",
                },
            },
            {
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "hash": target_hash,
                    "chainId": "0x7a69",
                    "gas": "0x5208",
                    "gasPrice": "0x1",
                    "r": "0x1",
                    "s": "0x1",
                    "type": "0x2",
                    "v": "0x1",
                    "yParity": "0x1",
                },
            },
        ]);

        canonicalize_monad_system_response(&requests, &mut responses, &target_hash);

        assert_eq!(responses[0]["result"]["gasUsed"], "0x0");
        assert_eq!(responses[1]["result"]["gas"], "0x0");
        assert_eq!(responses[1]["result"]["from"], MONAD_SYSTEM_ADDRESS);
        assert_eq!(responses[1]["result"]["type"], "0x0");
        assert_eq!(responses[1]["result"]["r"], "0x1");
        assert_eq!(responses[1]["result"]["s"], "0x1");
        assert_eq!(responses[1]["result"]["v"], "0xf4f6");
        assert!(responses[1]["result"].get("yParity").is_none());
    }

    #[test]
    fn canonical_monad_system_response_ignores_malformed_requests() {
        let request = json!({"jsonrpc": "2.0", "id": 1});
        let mut response = json!({"jsonrpc": "2.0", "id": 1, "result": "unchanged"});

        canonicalize_monad_system_response(&request, &mut response, &B256::ZERO.to_string());

        assert_eq!(response["result"], "unchanged");
    }

    #[tokio::test]
    #[ignore = "run manually"]
    async fn test_etherscan_keys() {
        let address = address!("0xdAC17F958D2ee523a2206206994597C13D831ec7");
        let mut first_abi = None;
        let mut failed = Vec::new();
        for (i, &key) in ETHERSCAN_KEYS.list.iter().enumerate() {
            println!("trying key {i} ({key})");

            let client = foundry_block_explorers::Client::builder()
                .chain(Chain::mainnet())
                .unwrap()
                .with_api_key(key)
                .build()
                .unwrap();

            let mut fail = |e: &str| {
                eprintln!("key {i} ({key}) failed: {e}");
                failed.push(key);
            };

            let abi = match client.contract_abi(address).await {
                Ok(abi) => abi,
                Err(e) => {
                    fail(&e.to_string());
                    continue;
                }
            };

            if let Some(first_abi) = &first_abi {
                if abi != *first_abi {
                    fail("abi mismatch");
                }
            } else {
                first_abi = Some(abi);
            }
        }
        assert!(failed.is_empty(), "failed keys: {failed:#?}")
    }
}
