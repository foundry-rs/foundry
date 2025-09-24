//! # foundry-evm-precompiles
//!
//! Foundry EVM network custom precompiles.

use crate::celo::transfer::{
    CELO_TRANSFER_ADDRESS, CELO_TRANSFER_LABEL, PRECOMPILE_ID_CELO_TRANSFER,
};
use alloy_chains::NamedChain;
use alloy_evm::precompiles::PrecompilesMap;
use alloy_primitives::{Address, map::AddressHashMap};
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

    pub fn with_chain_id(mut self, chain_id: u64) -> Self {
        if let Ok(NamedChain::Celo | NamedChain::CeloSepolia) = NamedChain::try_from(chain_id) {
            self.celo = true;
        }
        self
    }

    /// Inject precompiles for configured networks.
    pub fn inject_precompiles(self, precompiles: &mut PrecompilesMap) {
        if self.celo {
            precompiles.apply_precompile(&CELO_TRANSFER_ADDRESS, move |_| {
                Some(celo::transfer::precompile())
            });
        }
    }

    /// Returns precompiles label for configured networks, to be used in traces.
    pub fn precompiles_label(self) -> AddressHashMap<String> {
        let mut labels = AddressHashMap::default();
        if self.celo {
            labels.insert(CELO_TRANSFER_ADDRESS, CELO_TRANSFER_LABEL.to_string());
        }
        labels
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
