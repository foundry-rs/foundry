//! # foundry-evm-precompiles
//!
//! Foundry EVM network custom precompiles.

use crate::celo::transfer::{CELO_TRANSFER_ADDRESS, PRECOMPILE_ID_CELO_TRANSFER};
use alloy_evm::precompiles::PrecompilesMap;
use alloy_primitives::Address;
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod celo;

#[derive(Clone, Debug, Default, Parser, Copy, Serialize, Deserialize)]
pub struct NetworkConfigs {
    /// Enable Optimism network features.
    #[arg(help_heading = "Networks", long, visible_alias = "optimism")]
    // Skipped from configs (forge) as there is no feature to be added yet.
    #[serde(skip)]
    pub optimism: bool,
    /// Enable Celo network features.
    #[arg(help_heading = "Networks", long)]
    pub celo: bool,
}

impl NetworkConfigs {
    pub fn celo(mut self, celo: bool) -> Self {
        self.celo = celo;
        self
    }

    pub fn with_optimism() -> Self {
        Self { optimism: true, ..Default::default() }
    }

    /// Inject precompiles for configured networks.
    pub fn inject_precompiles(self, precompiles: &mut PrecompilesMap) {
        if self.celo {
            precompiles.apply_precompile(&CELO_TRANSFER_ADDRESS, move |_| {
                Some(celo::transfer::precompile())
            });
        }
    }

    /// Returns precompiles for configured networks.
    pub fn precompiles(self) -> BTreeMap<String, Address> {
        let mut precompiles = BTreeMap::new();
        if self.celo {
            precompiles
                .insert(PRECOMPILE_ID_CELO_TRANSFER.name().to_string(), CELO_TRANSFER_ADDRESS);
        }
        precompiles
    }
}
