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

#[derive(Default)]
pub struct NetworkPrecompiles {
    /// Whether to inject Odyssey precompiles.
    odyssey: bool,
    /// Whether to inject Celo precompiles.
    celo: bool,
}

impl NetworkPrecompiles {
    pub fn odyssey(mut self, odyssey: bool) -> Self {
        self.odyssey = odyssey;
        self
    }

    pub fn celo(mut self, celo: bool) -> Self {
        self.celo = celo;
        self
    }

    /// Inject precompiles for configured networks.
    pub fn inject(self, precompiles: &mut PrecompilesMap) {
        if self.odyssey {
            precompiles.apply_precompile(P256VERIFY.address(), move |_| {
                Some(DynPrecompile::from(move |input: PrecompileInput<'_>| {
                    P256VERIFY.precompile()(input.data, P256VERIFY_BASE_GAS_FEE)
                }))
            });
        }

        if self.celo {
            precompiles.apply_precompile(&CELO_TRANSFER_ADDRESS, move |_| {
                Some(celo::transfer::precompile())
            });
        }
    }

    /// Returns precompiles for configured networks.
    pub fn get(self) -> BTreeMap<String, Address> {
        let mut precompiles = BTreeMap::new();
        if self.odyssey {
            precompiles.insert(
                PrecompileId::P256Verify.name().to_string(),
                u64_to_address(P256VERIFY_ADDRESS),
            );
        }

        if self.celo {
            precompiles
                .insert(PRECOMPILE_ID_CELO_TRANSFER.name().to_string(), CELO_TRANSFER_ADDRESS);
        }
        precompiles
    }
}
