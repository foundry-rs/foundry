//! # foundry-evm-networks
//!
//! Foundry EVM network configuration.

use crate::{
    celo::transfer::{CELO_TRANSFER_ADDRESS, CELO_TRANSFER_LABEL, PRECOMPILE_ID_CELO_TRANSFER},
    tempo::TEMPO_PRECOMPILES,
};
use alloy_chains::{
    NamedChain,
    NamedChain::{Chiado, Gnosis, Moonbase, Moonbeam, MoonbeamDev, Moonriver, Rsk, RskTestnet},
};
use alloy_eips::eip1559::BaseFeeParams;
use alloy_evm::precompiles::PrecompilesMap;
use alloy_op_hardforks::{OpChainHardforks, OpHardforks};
use alloy_primitives::{Address, map::AddressHashMap};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod celo;
pub mod tempo;

/// Represents the active chain variant for EVM execution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChainVariant {
    #[default]
    Ethereum,
    Optimism,
    Celo,
    Tempo,
}

#[derive(Clone, Debug, Default, Parser, Copy, Serialize, Deserialize, PartialEq)]
pub struct NetworkConfigs {
    /// Enable Optimism network features.
    #[arg(help_heading = "Networks", long, conflicts_with = "celo")]
    // Skipped from configs (forge) as there is no feature to be added yet.
    #[serde(skip)]
    optimism: bool,
    /// Enable Celo network features.
    #[arg(help_heading = "Networks", long, conflicts_with = "optimism")]
    #[serde(default)]
    celo: bool,
    /// Enable Tempo network features.
    #[arg(help_heading = "Networks", long)]
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

    pub fn is_optimism(&self) -> bool {
        self.optimism
    }

    pub fn is_tempo(&self) -> bool {
        self.tempo
    }

    /// Returns the active chain variant based on network flags.
    /// Priority: Tempo > Optimism > Celo > Ethereum (default)
    pub fn variant(&self) -> ChainVariant {
        if self.tempo {
            ChainVariant::Tempo
        } else if self.optimism {
            ChainVariant::Optimism
        } else if self.celo {
            ChainVariant::Celo
        } else {
            ChainVariant::Ethereum
        }
    }

    /// Returns the base fee parameters for the configured network.
    ///
    /// For Optimism networks, returns Canyon parameters if the Canyon hardfork is active
    /// at the given timestamp, otherwise returns pre-Canyon parameters.
    pub fn base_fee_params(&self, timestamp: u64) -> BaseFeeParams {
        match self.variant() {
            ChainVariant::Optimism => {
                let op_hardforks = OpChainHardforks::op_mainnet();
                if op_hardforks.is_canyon_active_at_timestamp(timestamp) {
                    BaseFeeParams::optimism_canyon()
                } else {
                    BaseFeeParams::optimism()
                }
            }
            ChainVariant::Ethereum | ChainVariant::Celo | ChainVariant::Tempo => {
                BaseFeeParams::ethereum()
            }
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

    pub fn is_celo(&self) -> bool {
        self.celo
    }

    pub fn with_chain_id(mut self, chain_id: u64) -> Self {
        if let Ok(NamedChain::Celo | NamedChain::CeloSepolia) = NamedChain::try_from(chain_id) {
            self.celo = true;
        }
        // Tempo mainnet: 52014, Tempo testnet: 52015
        if chain_id == 52014 || chain_id == 52015 {
            self.tempo = true;
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
        if self.tempo {
            for precompile in TEMPO_PRECOMPILES {
                labels.insert(precompile.address(), precompile.name().to_string());
            }
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
        if self.tempo {
            for precompile in TEMPO_PRECOMPILES {
                precompiles.insert(precompile.name().to_string(), precompile.address());
            }
        }
        precompiles
    }
}
