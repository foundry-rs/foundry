//! Support rpc api keys

use once_cell::sync::Lazy;
use rand::seq::SliceRandom;
use std::sync::atomic::{AtomicUsize, Ordering};

// List of general purpose infura keys to rotate through
static INFURA_KEYS: Lazy<Vec<&'static str>> = Lazy::new(|| {
    let mut keys = vec![
        "6770454bc6ea42c58aac12978531b93f",
        "7a8769b798b642f6933f2ed52042bd70",
        "631fd9a6539644088297dc605d35fff3",
        "16a8be88795540b9b3903d8de0f7baa5",
        "f4a0bdad42674adab5fc0ac077ffab2b",
        "5c812e02193c4ba793f8c214317582bd",
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

pub fn next_rpc_endpoint(network: &str) -> String {
    let idx = next() % num_keys();
    if idx < INFURA_KEYS.len() {
        format!("https://{network}.infura.io/v3/{}", INFURA_KEYS[idx])
    } else {
        let idx = idx - INFURA_KEYS.len();
        format!("https://eth-{network}.alchemyapi.io/v2/{}", ALCHEMY_MAINNET_KEYS[idx])
    }
}

/// Returns endpoint that has access to archive state
pub fn next_http_archive_rpc_endpoint() -> String {
    let idx = next() % ALCHEMY_MAINNET_KEYS.len();
    format!("https://eth-mainnet.alchemyapi.io/v2/{}", ALCHEMY_MAINNET_KEYS[idx])
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
