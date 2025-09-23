//! RPC API keys utilities.

use foundry_config::{
    NamedChain,
    NamedChain::{
        Arbitrum, Base, BinanceSmartChainTestnet, Celo, Mainnet, Optimism, Polygon, Sepolia,
    },
};
use rand::seq::SliceRandom;
use std::sync::{
    LazyLock,
    atomic::{AtomicUsize, Ordering},
};

fn shuffled<T>(mut vec: Vec<T>) -> Vec<T> {
    vec.shuffle(&mut rand::rng());
    vec
}

macro_rules! shuffled_list {
    ($name:ident, $e:expr $(,)?) => {
        static $name: LazyLock<Vec<&'static str>> = LazyLock::new(|| shuffled($e));
    };
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
        "reth-ethereum-full.ithaca.xyz/rpc",
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
        "reth-ethereum-full.ithaca.xyz/ws",
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
fn next_archive_url(is_ws: bool) -> String {
    let domain = next(if is_ws { &WS_ARCHIVE_DOMAINS } else { &HTTP_ARCHIVE_DOMAINS });
    let url = if is_ws { format!("wss://{domain}") } else { format!("https://{domain}") };
    test_debug!("next_archive_url(is_ws={is_ws}) = {url}");
    url
}

/// Returns the next etherscan api key.
pub fn next_etherscan_api_key() -> String {
    let key = next(&ETHERSCAN_KEYS).to_string();
    test_debug!("next_etherscan_api_key() = {key}");
    key
}

fn next_url(is_ws: bool, chain: NamedChain) -> String {
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

    let reth_works = true;
    let domain = if reth_works && matches!(chain, Mainnet) {
        *next(if is_ws { &WS_DOMAINS } else { &HTTP_DOMAINS })
    } else {
        // DRPC for other networks used in tests.
        let key = next(&DRPC_KEYS);

        let network = match chain {
            Mainnet => "ethereum",
            Arbitrum => "arbitrum",
            Polygon => "polygon",
            Sepolia => "sepolia",
            _ => "",
        };
        &format!("lb.drpc.org/ogrpc?network={network}&dkey={key}")
    };

    let url = if is_ws { format!("wss://{domain}") } else { format!("https://{domain}") };

    test_debug!("next_url(is_ws={is_ws}, chain={chain:?}) = {url}");
    url
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
        for (i, &key) in ETHERSCAN_KEYS.iter().enumerate() {
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
