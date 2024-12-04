//! RPC API keys utilities.

use foundry_config::{NamedChain, NamedChain::Optimism};
use rand::seq::SliceRandom;
use std::{
    env,
    sync::{
        atomic::{AtomicUsize, Ordering},
        LazyLock,
    },
};

/// Env var key for ws archive endpoints.
const ENV_WS_ARCHIVE_ENDPOINTS: &str = "WS_ARCHIVE_URLS";
/// Env var key for http archive endpoints.
const ENV_HTTP_ARCHIVE_ENDPOINTS: &str = "HTTP_ARCHIVE_URLS";

// List of general purpose infura keys to rotate through
static INFURA_KEYS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut keys = vec![
        // "6cb19d07ca2d44f59befd61563b1037b",
        // "6d46c0cca653407b861f3f93f7b0236a",
        // "69a36846dec146e3a2898429be60be85",
        // "16a8be88795540b9b3903d8de0f7baa5",
        // "f4a0bdad42674adab5fc0ac077ffab2b",
        // "5c812e02193c4ba793f8c214317582bd",
    ];

    keys.shuffle(&mut rand::thread_rng());

    keys
});

// List of alchemy keys for mainnet
static ALCHEMY_KEYS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut keys = vec![
        "ib1f4u1ojm-9lJJypwkeZeG-75TJRB7O",
        "7mTtk6IW4DwroGnKmG_bOWri2hyaGYhX",
        "GL4M0hfzSYGU5e1_t804HoUDOObWP-FA",
        "WV407BEiBmjNJfKo9Uo_55u0z0ITyCOX",
        "Ge56dH9siMF4T0whP99sQXOcr2mFs8wZ",
        // "QC55XC151AgkS3FNtWvz9VZGeu9Xd9lb",
        // "pwc5rmJhrdoaSEfimoKEmsvOjKSmPDrP",
        // "A5sZ85MIr4SzCMkT0zXh2eeamGIq3vGL",
        // "9VWGraLx0tMiSWx05WH-ywgSVmMxs66W",
        // "U4hsGWgl9lBM1j3jhSgJ4gbjHg2jRwKy",
        "K-uNlqYoYCO9cdBHcifwCDAcEjDy1UHL",
        "GWdgwabOE2XfBdLp_gIq-q6QHa7DSoag",
        "Uz0cF5HCXFtpZlvd9NR7kHxfB_Wdpsx7",
        "wWZMf1SOu9lT1GNIJHOX-5WL1MiYXycT",
        "HACxy4wNUoD-oLlCq_v5LG0bclLc_DRL",
        "_kCjfMjYo8x0rOm6YzmvSI0Qk-c8SO5I",
        "kD-M-g5TKb957S3bbOXxXPeMUxm1uTuU",
        "jQqqfTOQN_7A6gQEjzRYpVwXzxEBN9aj",
        "jGiK5vwDfC3F4r0bqukm-W2GqgdrxdSr",
        "Reoz-NZSjWczcAQOeVTz_Ejukb8mAton",
        "-DQx9U-heCeTgYsAXwaTurmGytc-0mbR",
        "sDNCLu_e99YZRkbWlVHiuM3BQ5uxYCZU",
        "M6lfpxTBrywHOvKXOS4yb7cTTpa25ZQ9",
        "UK8U_ogrbYB4lQFTGJHHDrbiS4UPnac6",
        "Lc7oIGYeL_QvInzI0Wiu_pOZZDEKBrdf",
        "UVatYU2Ax0rX6bDiqddeTRDdcCxzdpoE",
        "bVjX9v-FpmUhf5R_oHIgwJx2kXvYPRbx",
    ];
    keys.shuffle(&mut rand::thread_rng());
    keys
});

// List of etherscan keys for mainnet
static ETHERSCAN_MAINNET_KEYS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut keys = vec![
        "MCAUM7WPE9XP5UQMZPCKIBUJHPM1C24FP6",
        "JW6RWCG2C5QF8TANH4KC7AYIF1CX7RB5D1",
        "ZSMDY6BI2H55MBE3G9CUUQT4XYUDBB6ZSK",
        "4FYHTY429IXYMJNS4TITKDMUKW5QRYDX61",
        "QYKNT5RHASZ7PGQE68FNQWH99IXVTVVD2I",
        "VXMQ117UN58Y4RHWUB8K1UGCEA7UQEWK55",
        "C7I2G4JTA5EPYS42Z8IZFEIMQNI5GXIJEV",
        "A15KZUMZXXCK1P25Y1VP1WGIVBBHIZDS74",
        "3IA6ASNQXN8WKN7PNFX7T72S9YG56X9FPG",
        "ZUB97R31KSYX7NYVW6224Q6EYY6U56H591",
        // Optimism
        // "JQNGFHINKS1W7Y5FRXU4SPBYF43J3NYK46",
    ];
    keys.shuffle(&mut rand::thread_rng());
    keys
});

// List of etherscan keys for Optimism.
static ETHERSCAN_OPTIMISM_KEYS: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| vec!["JQNGFHINKS1W7Y5FRXU4SPBYF43J3NYK46"]);

/// Returns the next index to use.
fn next_idx() -> usize {
    static NEXT_INDEX: AtomicUsize = AtomicUsize::new(0);
    NEXT_INDEX.fetch_add(1, Ordering::SeqCst)
}

/// Returns the next item in the list to use.
fn next<T>(list: &[T]) -> &T {
    &list[next_idx() % list.len()]
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
///
/// Uses either environment variables (comma separated urls) or default keys.
fn next_archive_url(is_ws: bool) -> String {
    let urls = archive_urls(is_ws);
    let url = if env_archive_urls(is_ws).is_empty() {
        next(urls)
    } else {
        urls.choose_weighted(&mut rand::thread_rng(), |url| {
            if url.contains("reth") {
                2usize
            } else {
                1usize
            }
        })
        .unwrap()
    };
    eprintln!("--- next_archive_url(is_ws={is_ws}) = {url} ---");
    url.clone()
}

fn archive_urls(is_ws: bool) -> &'static [String] {
    static WS: LazyLock<Vec<String>> = LazyLock::new(|| get(true));
    static HTTP: LazyLock<Vec<String>> = LazyLock::new(|| get(false));

    fn get(is_ws: bool) -> Vec<String> {
        let env_urls = env_archive_urls(is_ws);
        if !env_urls.is_empty() {
            let mut urls = env_urls.to_vec();
            urls.shuffle(&mut rand::thread_rng());
            return urls;
        }

        let mut urls = Vec::new();
        for &key in ALCHEMY_KEYS.iter() {
            if is_ws {
                urls.push(format!("wss://eth-mainnet.g.alchemy.com/v2/{key}"));
            } else {
                urls.push(format!("https://eth-mainnet.g.alchemy.com/v2/{key}"));
            }
        }
        urls
    }

    if is_ws {
        &WS
    } else {
        &HTTP
    }
}

fn env_archive_urls(is_ws: bool) -> &'static [String] {
    static WS: LazyLock<Vec<String>> = LazyLock::new(|| get(true));
    static HTTP: LazyLock<Vec<String>> = LazyLock::new(|| get(false));

    fn get(is_ws: bool) -> Vec<String> {
        let env = if is_ws { ENV_WS_ARCHIVE_ENDPOINTS } else { ENV_HTTP_ARCHIVE_ENDPOINTS };
        let env = env::var(env).unwrap_or_default();
        let env = env.trim();
        if env.is_empty() {
            return vec![];
        }
        env.split(',').map(str::trim).filter(|s| !s.is_empty()).map(ToString::to_string).collect()
    }

    if is_ws {
        &WS
    } else {
        &HTTP
    }
}

/// Returns the next etherscan api key
pub fn next_mainnet_etherscan_api_key() -> String {
    next_etherscan_api_key(NamedChain::Mainnet)
}

/// Returns the next etherscan api key for given chain.
pub fn next_etherscan_api_key(chain: NamedChain) -> String {
    let keys = match chain {
        Optimism => &ETHERSCAN_OPTIMISM_KEYS,
        _ => &ETHERSCAN_MAINNET_KEYS,
    };
    let key = next(keys).to_string();
    eprintln!("--- next_etherscan_api_key(chain={chain:?}) = {key} ---");
    key
}

fn next_url(is_ws: bool, chain: NamedChain) -> String {
    use NamedChain::*;

    if matches!(chain, NamedChain::Base) {
        return "https://mainnet.base.org".to_string();
    }

    let idx = next_idx() % (INFURA_KEYS.len() + ALCHEMY_KEYS.len());
    let is_infura = idx < INFURA_KEYS.len();

    let key = if is_infura { INFURA_KEYS[idx] } else { ALCHEMY_KEYS[idx - INFURA_KEYS.len()] };

    // Nowhere near complete.
    let prefix = if is_infura {
        match chain {
            Optimism => "optimism",
            Arbitrum => "arbitrum",
            Polygon => "polygon",
            _ => "",
        }
    } else {
        match chain {
            Optimism => "opt",
            Arbitrum => "arb",
            Polygon => "polygon",
            _ => "eth",
        }
    };
    let network = if is_infura {
        match chain {
            Mainnet | Optimism | Arbitrum | Polygon => "mainnet",
            _ => chain.as_str(),
        }
    } else {
        match chain {
            Mainnet | Optimism | Arbitrum | Polygon => "mainnet",
            _ => chain.as_str(),
        }
    };
    let full = if prefix.is_empty() { network.to_string() } else { format!("{prefix}-{network}") };

    let url = match (is_ws, is_infura) {
        (false, true) => format!("https://{full}.infura.io/v3/{key}"),
        (true, true) => format!("wss://{full}.infura.io/ws/v3/{key}"),
        (false, false) => format!("https://{full}.g.alchemy.com/v2/{key}"),
        (true, false) => format!("wss://{full}.g.alchemy.com/v2/{key}"),
    };
    eprintln!("--- next_url(is_ws={is_ws}, chain={chain:?}) = {url} ---");
    url
}

#[cfg(test)]
#[allow(clippy::needless_return, clippy::disallowed_macros)]
mod tests {
    use super::*;
    use alloy_primitives::address;
    use foundry_config::Chain;

    #[tokio::test]
    #[ignore = "run manually"]
    async fn test_etherscan_keys() {
        let address = address!("dAC17F958D2ee523a2206206994597C13D831ec7");
        let mut first_abi = None;
        let mut failed = Vec::new();
        for (i, &key) in ETHERSCAN_MAINNET_KEYS.iter().enumerate() {
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
