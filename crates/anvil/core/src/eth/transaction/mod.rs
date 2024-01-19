//! Transaction related types

pub mod alloy;
pub mod alloy_compat;
pub mod optimism;

/// The signature used to bypass signing via the `eth_sendUnsignedTransaction` cheat RPC
#[cfg(feature = "impersonated-tx")]
pub const IMPERSONATED_SIGNATURE: alloy_rpc_types::Signature = alloy_rpc_types::Signature {
    r: alloy_primitives::U256::ZERO,
    s: alloy_primitives::U256::ZERO,
    v: alloy_primitives::U256::ZERO,
    y_parity: None,
};
