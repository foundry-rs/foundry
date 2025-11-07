//! RPC API keys utilities.

use foundry_config::{
    NamedChain::{
        self, Arbitrum, Base, BinanceSmartChainTestnet, Celo, Mainnet, Optimism, Polygon, Sepolia,
    },
    RpcEndpointUrl, RpcEndpoints,
};
use rand::seq::SliceRandom;
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
        "reth-ethereum.ithaca.xyz/rpc",
    ],
);
shuffled_list!(
    HTTP_DOMAINS,
    vec![
        //
        "reth-ethereum.ithaca.xyz/rpc",
        // "reth-ethereum-full.ithaca.xyz/rpc",
    ],
);
shuffled_list!(
    WS_ARCHIVE_DOMAINS,
    vec![
        //
        "reth-ethereum.ithaca.xyz/ws",
    ],
);
shuffled_list!(
    WS_DOMAINS,
    vec![
        //
        "reth-ethereum.ithaca.xyz/ws",
        // "reth-ethereum-full.ithaca.xyz/ws",
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
        ("arbitrum", RpcEndpointUrl::Url(next_rpc_endpoint(NamedChain::Arbitrum))),
        ("polygon", RpcEndpointUrl::Url(next_rpc_endpoint(NamedChain::Polygon))),
        ("bsc", RpcEndpointUrl::Url(next_rpc_endpoint(NamedChain::BinanceSmartChain))),
        ("avaxTestnet", RpcEndpointUrl::Url("https://api.avax-test.network/ext/bc/C/rpc".into())),
        ("moonbeam", RpcEndpointUrl::Url("https://moonbeam-rpc.publicnode.com".into())),
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

/// Returns a websocket URL that has access to archive state
pub fn next_http_archive_rpc_url() -> String {
    next_archive_url(false)
}

/// Returns an HTTP URL that has access to archive state
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

#[cfg(test)]
#[expect(clippy::disallowed_macros)]
mod tests {
    use super::*;
    use alloy_primitives::address;
    use foundry_config::Chain;

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
        if !failed.is_empty() {
            panic!("failed keys: {failed:#?}");
        }
    }
}
