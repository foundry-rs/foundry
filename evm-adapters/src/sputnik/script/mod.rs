//! support for writing scripts with solidity

use ethers_core::types::Address;
use once_cell::sync::Lazy;

pub mod handler;

/// Address where the forge script vm listens for
// `Address::from_slice(&keccak256("forge sol script")[12..])`
pub static FORGE_SCRIPT_ADDRESS: Lazy<Address> = Lazy::new(|| {
    Address::from_slice(&hex::decode("cc72bd077e2b77a8eee22a99520a6a503a73dc65").unwrap())
});
