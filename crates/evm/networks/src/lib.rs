//! # foundry-evm-networks
//!
//! Foundry EVM network configuration.

use crate::celo::transfer::{
    CELO_TRANSFER_ADDRESS, CELO_TRANSFER_LABEL, PRECOMPILE_ID_CELO_TRANSFER,
};
use alloy_chains::{
    Chain, NamedChain,
    NamedChain::{Chiado, Gnosis, Moonbase, Moonbeam, MoonbeamDev, Moonriver, Rsk, RskTestnet},
};
use alloy_eips::eip1559::BaseFeeParams;
use alloy_evm::precompiles::PrecompilesMap;
use alloy_op_hardforks::{OpChainHardforks, OpHardforks};
use alloy_primitives::{Address, map::AddressHashMap};
use clap::Parser;
use foundry_evm_hardforks::FoundryHardfork;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod celo;

#[derive(Clone, Debug, Default, Parser, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkConfigs {
    /// Enable Optimism network features.
    #[arg(help_heading = "Networks", long, conflicts_with_all = ["celo", "tempo"])]
    // Skipped from configs (forge) as there is no feature to be added yet.
    #[serde(skip)]
    optimism: bool,
    /// Enable Celo network features.
    #[arg(help_heading = "Networks", long, conflicts_with_all = ["optimism", "tempo"])]
    #[serde(default)]
    celo: bool,
    /// Enable Tempo network features.
    #[arg(help_heading = "Networks", long, conflicts_with_all = ["optimism", "celo"])]
    #[serde(default)]
    tempo: bool,
    /// Whether to bypass prevrandao.
    #[arg(skip)]
    #[serde(default)]
    bypass_prevrandao: bool,
}

impl NetworkConfigs {
    pub fn with_optimism() -> Self {
        Self { optimism: true, ..Default::default() }
    }

    pub fn with_celo() -> Self {
        Self { celo: true, ..Default::default() }
    }

    pub fn with_tempo() -> Self {
        Self { tempo: true, ..Default::default() }
    }

    pub const fn is_optimism(&self) -> bool {
        self.optimism
    }

    pub const fn is_tempo(&self) -> bool {
        self.tempo
    }

    /// Returns the base fee parameters for the configured network.
    ///
    /// For Optimism networks, returns Canyon parameters if the Canyon hardfork is active
    /// at the given timestamp, otherwise returns pre-Canyon parameters.
    pub fn base_fee_params(&self, timestamp: u64) -> BaseFeeParams {
        if self.is_optimism() {
            let op_hardforks = OpChainHardforks::op_mainnet();
            if op_hardforks.is_canyon_active_at_timestamp(timestamp) {
                BaseFeeParams::optimism_canyon()
            } else {
                BaseFeeParams::optimism()
            }
        } else {
            BaseFeeParams::ethereum()
        }
    }

    pub fn bypass_prevrandao(&self, chain_id: u64) -> bool {
        if let Ok(
            Moonbeam | Moonbase | Moonriver | MoonbeamDev | Rsk | RskTestnet | Gnosis | Chiado,
        ) = NamedChain::try_from(chain_id)
        {
            return true;
        }
        self.bypass_prevrandao
    }

    pub const fn is_celo(&self) -> bool {
        self.celo
    }

    /// Returns the name of the currently active non-Ethereum network, or `None` for plain Ethereum.
    pub const fn active_network_name(&self) -> Option<&'static str> {
        if self.tempo {
            Some("tempo")
        } else if self.optimism {
            Some("optimism")
        } else if self.celo {
            Some("celo")
        } else {
            None
        }
    }

    pub fn with_chain_id(mut self, chain_id: u64) -> Self {
        // Only infer network if no explicit network is already set
        if !self.celo && !self.tempo && !self.optimism {
            let chain = Chain::from_id(chain_id);
            if matches!(chain.named(), Some(NamedChain::Celo | NamedChain::CeloSepolia)) {
                self.celo = true;
            } else if chain.is_tempo() {
                self.tempo = true;
            } else if chain.is_optimism() {
                self.optimism = true;
            }
        }
        self
    }

    /// Validates `hardfork` against the current `NetworkConfigs` and, if consistent, returns an
    /// updated instance with the network implied by the enabled hardfork.
    ///
    /// Returns `Err` when the hardfork's network family conflicts with the configured one.
    pub fn normalize_for_hardfork(self, hardfork: FoundryHardfork) -> Result<Self, String> {
        if let Some(configured) =
            self.active_network_name().filter(|&n| Some(n) != hardfork.namespace())
        {
            return Err(format!(
                "hardfork `{}` conflicts with network config `{configured}`",
                String::from(hardfork),
            ));
        }

        let network = match hardfork {
            FoundryHardfork::Ethereum(_) => self,
            FoundryHardfork::Tempo(_) => Self::with_tempo(),
            FoundryHardfork::Optimism(_) => Self::with_optimism(),
        };

        Ok(network)
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
