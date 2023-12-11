//! RPC API keys utilities.

use once_cell::sync::Lazy;
use rand::seq::SliceRandom;
use std::sync::atomic::{AtomicUsize, Ordering};

// List of general purpose infura keys to rotate through
static INFURA_KEYS: Lazy<Vec<&'static str>> = Lazy::new(|| {
    let mut keys = vec![
        // "16a8be88795540b9b3903d8de0f7baa5",
        // "f4a0bdad42674adab5fc0ac077ffab2b",
        // "5c812e02193c4ba793f8c214317582bd",
    ];

    keys.shuffle(&mut rand::thread_rng());

    keys
});

// List of alchemy keys for mainnet
static ALCHEMY_MAINNET_KEYS: Lazy<Vec<&'static str>> = Lazy::new(|| {
    let mut keys = vec![
        "ib1f4u1ojm-9lJJypwkeZeG-75TJRB7O",
        "7mTtk6IW4DwroGnKmG_bOWri2hyaGYhX",
        "GL4M0hfzSYGU5e1_t804HoUDOObWP-FA",
        "WV407BEiBmjNJfKo9Uo_55u0z0ITyCOX",
        "Ge56dH9siMF4T0whP99sQXOcr2mFs8wZ",
        "QC55XC151AgkS3FNtWvz9VZGeu9Xd9lb",
        "pwc5rmJhrdoaSEfimoKEmsvOjKSmPDrP",
        "A5sZ85MIr4SzCMkT0zXh2eeamGIq3vGL",
        "9VWGraLx0tMiSWx05WH-ywgSVmMxs66W",
        "U4hsGWgl9lBM1j3jhSgJ4gbjHg2jRwKy",
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
    ];

    keys.shuffle(&mut rand::thread_rng());

    keys
});

/// counts how many times a rpc endpoint was requested for _mainnet_
static NEXT_RPC_ENDPOINT: AtomicUsize = AtomicUsize::new(0);

// returns the current value of the atomic counter and increments it
fn next() -> usize {
    NEXT_RPC_ENDPOINT.fetch_add(1, Ordering::SeqCst)
}

fn num_keys() -> usize {
    INFURA_KEYS.len() + ALCHEMY_MAINNET_KEYS.len()
}

/// Returns the next _mainnet_ rpc endpoint in inline
///
/// This will rotate all available rpc endpoints
pub fn next_http_rpc_endpoint() -> String {
    next_rpc_endpoint("mainnet")
}

/// Returns the next _mainnet_ rpc endpoint in inline
///
/// This will rotate all available rpc endpoints
pub fn next_ws_rpc_endpoint() -> String {
    next_ws_endpoint("mainnet")
}

/// Returns the next HTTP RPC endpoint.
pub fn next_rpc_endpoint(network: &str) -> String {
    let idx = next() % num_keys();
    if idx < INFURA_KEYS.len() {
        format!("https://{network}.infura.io/v3/{}", INFURA_KEYS[idx])
    } else {
        let idx = idx - INFURA_KEYS.len();
        format!("https://eth-{network}.alchemyapi.io/v2/{}", ALCHEMY_MAINNET_KEYS[idx])
    }
}

/// Returns the next WS RPC endpoint.
pub fn next_ws_endpoint(network: &str) -> String {
    let idx = next() % num_keys();
    if idx < INFURA_KEYS.len() {
        format!("wss://{network}.infura.io/v3/{}", INFURA_KEYS[idx])
    } else {
        let idx = idx - INFURA_KEYS.len();
        format!("wss://eth-{network}.alchemyapi.io/v2/{}", ALCHEMY_MAINNET_KEYS[idx])
    }
}

/// Returns endpoint that has access to archive state
pub fn next_http_archive_rpc_endpoint() -> String {
    let idx = next() % ALCHEMY_MAINNET_KEYS.len();
    format!("https://eth-mainnet.alchemyapi.io/v2/{}", ALCHEMY_MAINNET_KEYS[idx])
}

/// Returns endpoint that has access to archive state
pub fn next_ws_archive_rpc_endpoint() -> String {
    let idx = next() % ALCHEMY_MAINNET_KEYS.len();
    format!("wss://eth-mainnet.alchemyapi.io/v2/{}", ALCHEMY_MAINNET_KEYS[idx])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    #[ignore]
    fn can_rotate_unique() {
        let mut keys = HashSet::new();
        for _ in 0..100 {
            keys.insert(next_http_rpc_endpoint());
        }
        assert_eq!(keys.len(), num_keys());
    }
}
