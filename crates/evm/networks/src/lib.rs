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
use alloy_primitives::{Address, ChainId, map::AddressHashMap};
use clap::Parser;
use foundry_evm_hardforks::FoundryHardfork;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

pub mod celo;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lowercase")]
pub enum NetworkVariant {
    #[default]
    Ethereum,
    Optimism,
    Tempo,
}

impl NetworkVariant {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Ethereum => "ethereum",
            Self::Optimism => "optimism",
            Self::Tempo => "tempo",
        }
    }
}

impl std::fmt::Display for NetworkVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl From<ChainId> for NetworkVariant {
    fn from(chain_id: ChainId) -> Self {
        let chain = Chain::from_id(chain_id);
        if chain.is_tempo() {
            Self::Tempo
        } else if chain.is_optimism() {
            Self::Optimism
        } else {
            Self::Ethereum
        }
    }
}

#[derive(Clone, Debug, Default, Parser, Copy, PartialEq, Eq)]
pub struct NetworkConfigs {
    /// Enable a specific network family.
    #[arg(help_heading = "Networks", long, short, num_args = 1, value_name = "NETWORK", value_enum, conflicts_with_all = ["celo", "optimism", "tempo"])]
    network: Option<NetworkVariant>,
    /// Enable Celo network features.
    #[arg(help_heading = "Networks", long, conflicts_with_all = ["optimism", "tempo"])]
    celo: bool,
    /// Enable Optimism network features (deprecated: use --network optimism).
    #[arg(long, hide = true, conflicts_with_all = ["celo", "tempo"])]
    optimism: bool,
    /// Enable Tempo network features (deprecated: use --network tempo).
    #[arg(long, hide = true, conflicts_with_all = ["celo", "optimism"])]
    tempo: bool,
    /// Whether to bypass prevrandao.
    #[arg(skip)]
    bypass_prevrandao: bool,
}

/// Serialize with backward-compatible `optimism` / `tempo` boolean fields
/// derived from the `network` field.
impl Serialize for NetworkConfigs {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("NetworkConfigs", 4)?;
        s.serialize_field("celo", &self.is_celo())?;
        s.serialize_field("tempo", &self.is_tempo())?;
        s.serialize_field("bypass_prevrandao", &self.bypass_prevrandao)?;
        s.end()
    }
}

/// Custom deserializer that supports both the new `network = "tempo"` format
/// and legacy `tempo = true` / `optimism = true` boolean fields.
impl<'de> Deserialize<'de> for NetworkConfigs {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            network: Option<NetworkVariant>,
            #[serde(default)]
            celo: bool,
            #[serde(default)]
            bypass_prevrandao: bool,
            // Legacy boolean fields
            #[serde(default)]
            optimism: bool,
            #[serde(default)]
            tempo: bool,
        }

        let raw = Raw::deserialize(deserializer)?;

        let network = match raw.network {
            Some(n) => Some(n),
            None if raw.tempo => Some(NetworkVariant::Tempo),
            None if raw.optimism => Some(NetworkVariant::Optimism),
            None => None,
        };

        Ok(Self {
            network,
            celo: raw.celo,
            optimism: false,
            tempo: false,
            bypass_prevrandao: raw.bypass_prevrandao,
        })
    }
}

impl NetworkConfigs {
    /// Resolves deprecated `--optimism`/`--tempo` flags into the `network` field.
    pub const fn resolved(mut self) -> Self {
        if self.network.is_none() {
            if self.optimism {
                self.network = Some(NetworkVariant::Optimism);
            } else if self.tempo {
                self.network = Some(NetworkVariant::Tempo);
            }
        }
        self
    }

    pub fn with_optimism() -> Self {
        Self { network: Some(NetworkVariant::Optimism), ..Default::default() }
    }

    pub fn with_celo() -> Self {
        Self { celo: true, ..Default::default() }
    }

    pub fn with_tempo() -> Self {
        Self { network: Some(NetworkVariant::Tempo), ..Default::default() }
    }

    pub fn is_optimism(&self) -> bool {
        matches!(self.resolved_network(), Some(NetworkVariant::Optimism))
    }

    pub fn is_tempo(&self) -> bool {
        matches!(self.resolved_network(), Some(NetworkVariant::Tempo))
    }

    pub const fn is_celo(&self) -> bool {
        self.celo
    }

    /// Returns the resolved network variant, folding legacy flags.
    fn resolved_network(&self) -> Option<NetworkVariant> {
        self.network.or(if self.optimism {
            Some(NetworkVariant::Optimism)
        } else if self.tempo {
            Some(NetworkVariant::Tempo)
        } else {
            None
        })
    }

    /// Returns the name of the currently active non-Ethereum network, or `None` for plain Ethereum.
    pub fn active_network_name(&self) -> Option<&'static str> {
        self.resolved_network().and_then(|n| match n {
            NetworkVariant::Ethereum => None,
            _ => Some(n.name()),
        })
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

    pub fn with_chain_id(mut self, chain_id: u64) -> Self {
        if self.resolved_network().is_none() {
            let chain = Chain::from_id(chain_id);
            if chain.is_tempo() {
                self.network = Some(NetworkVariant::Tempo);
            } else if chain.is_optimism() {
                self.network = Some(NetworkVariant::Optimism);
            }
        }
        if !self.celo {
            let chain = Chain::from_id(chain_id);
            if matches!(chain.named(), Some(NamedChain::Celo | NamedChain::CeloSepolia)) {
                self.celo = true;
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
