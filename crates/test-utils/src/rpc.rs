//! RPC testing utilities.

use alloy_primitives::B256;
use axum::{Json, Router, routing::post};
use foundry_config::{
    NamedChain::{
        self, Arbitrum, Base, BinanceSmartChainTestnet, Celo, Mainnet, Optimism, Polygon, Sepolia,
    },
    RpcEndpointUrl, RpcEndpoints,
};
use rand::seq::SliceRandom;
use serde_json::{Value, json};
use std::{
    env,
    sync::{
        LazyLock,
        atomic::{AtomicUsize, Ordering},
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
        ("moonbeam", RpcEndpointUrl::Url("https://moonbeam-rpc.publicnode.com".into())),
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
        return "https://bsc-testnet-rpc.publicnode.com".to_string();
    }

    if matches!(chain, Celo) {
        return "https://celo.drpc.org".to_string();
    }

    if matches!(chain, Sepolia) {
        let rpc_url = env::var("ETH_SEPOLIA_RPC").unwrap_or_default();
        if !rpc_url.is_empty() {
            return rpc_url;
        }
    }

    if matches!(chain, Arbitrum) {
        let rpc_url = env::var("ARBITRUM_RPC").unwrap_or_default();
        if !rpc_url.is_empty() {
            return rpc_url;
        }
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
            Arbitrum => "arbitrum",
            Sepolia => "sepolia",
            _ => "",
        };
        &format!("lb.drpc.org/ogrpc?network={network}&dkey={key}")
    };

    if is_ws { format!("wss://{domain}") } else { format!("https://{domain}") }
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
