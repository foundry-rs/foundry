//! # foundry-evm-precompiles
//!
//! Foundry EVM network custom precompiles.

use crate::celo::transfer::{CELO_TRANSFER_ADDRESS, PRECOMPILE_ID_CELO_TRANSFER};
use alloy_evm::precompiles::{DynPrecompile, PrecompileInput, PrecompilesMap};
use alloy_primitives::Address;
use revm::precompile::{
    PrecompileId,
    secp256r1::{P256VERIFY, P256VERIFY_ADDRESS, P256VERIFY_BASE_GAS_FEE},
    u64_to_address,
};
use std::collections::BTreeMap;

pub mod celo;

/// Conditionally inject network precompiles.
pub fn inject_network_precompiles(precompiles: &mut PrecompilesMap, odyssey: bool, celo: bool) {
    if odyssey {
        precompiles.apply_precompile(P256VERIFY.address(), move |_| {
            Some(DynPrecompile::from(move |input: PrecompileInput<'_>| {
                P256VERIFY.precompile()(input.data, P256VERIFY_BASE_GAS_FEE)
            }))
        });
    }

    if celo {
        precompiles
            .apply_precompile(&CELO_TRANSFER_ADDRESS, move |_| Some(celo::transfer::precompile()));
    }
}

pub fn map_network_precompiles(
    precompiles_map: &mut BTreeMap<String, Address>,
    odyssey: bool,
    celo: bool,
) {
    if odyssey {
        precompiles_map.insert(
            PrecompileId::P256Verify.name().to_string(),
            u64_to_address(P256VERIFY_ADDRESS),
        );
    }

    if celo {
        precompiles_map
            .insert(PRECOMPILE_ID_CELO_TRANSFER.name().to_string(), CELO_TRANSFER_ADDRESS);
    }
}
