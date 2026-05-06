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
use alloy_primitives::{Address, ChainId, map::AddressHashMap};
use clap::Parser;
use foundry_evm_hardforks::FoundryHardfork;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod celo;

#[cfg(feature = "optimism")]
mod optimism;

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lowercase")]
pub enum NetworkVariant {
    #[default]
    Ethereum,
    #[cfg(feature = "optimism")]
    Optimism,
    Tempo,
}

impl std::str::FromStr for NetworkVariant {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ethereum" => Ok(Self::Ethereum),
            #[cfg(feature = "optimism")]
            "optimism" => Ok(Self::Optimism),
            "tempo" => Ok(Self::Tempo),
            _ => Err(format!("unknown network variant: {s}")),
        }
    }
}

impl NetworkVariant {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Ethereum => "ethereum",
            #[cfg(feature = "optimism")]
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
            return Self::Tempo;
        }
        #[cfg(feature = "optimism")]
        if chain.is_optimism() {
            return Self::Optimism;
        }
        Self::Ethereum
    }
}

#[derive(Clone, Debug, Default, Parser, Deserialize, Copy, PartialEq, Eq)]
pub struct NetworkConfigs {
    /// Enable a specific network family.
    #[arg(help_heading = "Networks", long, short, num_args = 1, value_name = "NETWORK", value_enum, conflicts_with_all = ["celo", "tempo"])]
    #[cfg_attr(feature = "optimism", arg(conflicts_with = "optimism"))]
    #[serde(default)]
    pub(crate) network: Option<NetworkVariant>,
    /// Enable Celo network features.
    #[arg(help_heading = "Networks", long, conflicts_with_all = ["network", "tempo"])]
    #[cfg_attr(feature = "optimism", arg(conflicts_with = "optimism"))]
    celo: bool,
    /// Enable Optimism network features (deprecated: use --network optimism).
    #[cfg(feature = "optimism")]
    #[arg(long, hide = true, conflicts_with_all = ["network", "celo", "tempo"])]
    // Deserialize-only legacy alias: accepted in foundry.toml but never serialized — the
    // canonical form is `network = "optimism"`.
    #[serde(default)]
    pub(crate) optimism: bool,
    /// Enable Tempo network features (deprecated: use --network tempo).
    #[arg(long, hide = true, conflicts_with_all = ["network", "celo"])]
    #[cfg_attr(feature = "optimism", arg(conflicts_with = "optimism"))]
    // Deserialize-only legacy alias: accepted in foundry.toml but never serialized — the
    // canonical form is `network = "tempo"`.
    #[serde(default)]
    tempo: bool,
    /// Whether to bypass prevrandao.
    #[arg(skip)]
    #[serde(default)]
    bypass_prevrandao: bool,
}

// Custom `Serialize` impl: always emits the *resolved* network as the canonical
// `network = "..."` field, and never emits the legacy `tempo` / `optimism` aliases. This avoids
// confusing output like `network = "tempo"` next to `tempo = false`, and ensures `tempo = true`
// in foundry.toml round-trips as `network = "tempo"`.
impl Serialize for NetworkConfigs {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("NetworkConfigs", 3)?;
        s.serialize_field("network", &self.resolved_network())?;
        s.serialize_field("celo", &self.celo)?;
        s.serialize_field("bypass_prevrandao", &self.bypass_prevrandao)?;
        s.end()
    }
}

impl NetworkConfigs {
    pub fn with_celo() -> Self {
        Self { celo: true, ..Default::default() }
    }

    pub fn with_tempo() -> Self {
        Self { network: Some(NetworkVariant::Tempo), tempo: true, ..Default::default() }
    }

    pub const fn is_tempo(&self) -> bool {
        matches!(self.resolved_network(), Some(NetworkVariant::Tempo))
    }

    pub const fn is_celo(&self) -> bool {
        self.celo
    }

    /// Returns the resolved network variant, folding legacy flags.
    const fn resolved_network(&self) -> Option<NetworkVariant> {
        if let Some(n) = self.network {
            return Some(n);
        }
        #[cfg(feature = "optimism")]
        if self.optimism {
            return Some(NetworkVariant::Optimism);
        }
        if self.tempo {
            return Some(NetworkVariant::Tempo);
        }
        None
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
        #[cfg(feature = "optimism")]
        if self.is_optimism() {
            return self.op_base_fee_params(timestamp);
        }
        let _ = timestamp;
        BaseFeeParams::ethereum()
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

    pub fn with_chain_id(self, chain_id: u64) -> Self {
        let chain = Chain::from_id(chain_id);
        if self.resolved_network().is_some() {
            return if !self.celo
                && matches!(chain.named(), Some(NamedChain::Celo | NamedChain::CeloSepolia))
            {
                Self::with_celo()
            } else {
                self
            };
        }
        if chain.is_tempo() {
            return Self::with_tempo();
        }
        #[cfg(feature = "optimism")]
        if chain.is_optimism() {
            return Self::with_optimism();
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
            #[cfg(feature = "optimism")]
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

impl From<NetworkVariant> for NetworkConfigs {
    fn from(network: NetworkVariant) -> Self {
        match network {
            NetworkVariant::Ethereum => Self::default(),
            NetworkVariant::Tempo => {
                Self { network: Some(network), tempo: true, ..Default::default() }
            }
            #[cfg(feature = "optimism")]
            NetworkVariant::Optimism => {
                Self { network: Some(network), optimism: true, ..Default::default() }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Equivalence: new flag == legacy flag ---

    #[test]
    fn new_tempo_flag_equivalent_to_legacy() {
        let via_new = NetworkConfigs { network: Some(NetworkVariant::Tempo), ..Default::default() };
        let via_old = NetworkConfigs { tempo: true, ..Default::default() };
        assert_eq!(via_new.is_tempo(), via_old.is_tempo());
        assert_eq!(via_new.active_network_name(), via_old.active_network_name());
    }

    // --- resolved() / active_network_name ---

    #[test]
    fn active_network_name_tempo() {
        let cfg = NetworkConfigs::with_tempo();
        assert_eq!(cfg.active_network_name(), Some("tempo"));
    }

    #[test]
    fn active_network_name_default_is_none() {
        assert_eq!(NetworkConfigs::default().active_network_name(), None);
    }

    // --- Serde round-trip ---

    #[test]
    fn serde_roundtrip_tempo() {
        let original = NetworkConfigs::with_tempo();
        let json = serde_json::to_string(&original).unwrap();
        let restored: NetworkConfigs = serde_json::from_str(&json).unwrap();
        assert!(restored.is_tempo());
    }

    #[test]
    fn serde_legacy_tempo_bool_deserialized() {
        // Old foundry.toml format: `tempo = true`
        let json = r#"{"tempo": true, "celo": false, "bypass_prevrandao": false}"#;
        let cfg: NetworkConfigs = serde_json::from_str(json).unwrap();
        assert!(cfg.is_tempo());
    }

    #[test]
    fn serde_serializes_legacy_alias_as_canonical_network() {
        // Legacy `tempo = true` should serialize as the canonical `network = "tempo"`,
        // and the legacy `tempo` / `optimism` keys must not appear in the output.
        let cfg = NetworkConfigs { tempo: true, ..Default::default() };
        let json = serde_json::to_value(cfg).unwrap();
        assert_eq!(json["network"], serde_json::json!("tempo"));
        assert!(json.get("tempo").is_none(), "legacy `tempo` key should not be serialized");
        assert!(json.get("optimism").is_none(), "legacy `optimism` key should not be serialized");
    }

    #[test]
    fn serde_new_network_field_deserialized() {
        let json_tempo = r#"{"network": "tempo", "celo": false, "bypass_prevrandao": false}"#;
        let cfg_tempo: NetworkConfigs = serde_json::from_str(json_tempo).unwrap();
        assert!(cfg_tempo.is_tempo());
    }

    #[cfg(feature = "optimism")]
    mod optimism {
        use super::*;

        #[test]
        fn new_optimism_flag_equivalent_to_legacy() {
            let via_new =
                NetworkConfigs { network: Some(NetworkVariant::Optimism), ..Default::default() };
            let via_old = NetworkConfigs { optimism: true, ..Default::default() };
            assert_eq!(via_new.is_optimism(), via_old.is_optimism());
            assert_eq!(via_new.is_tempo(), via_old.is_tempo());
            assert_eq!(via_new.active_network_name(), via_old.active_network_name());
        }

        #[test]
        fn active_network_name_optimism() {
            let cfg = NetworkConfigs::with_optimism();
            assert_eq!(cfg.active_network_name(), Some("optimism"));
        }

        #[test]
        fn new_flag_wins_over_legacy_when_both_set() {
            // --network optimism --tempo: network field wins
            let cfg = NetworkConfigs {
                network: Some(NetworkVariant::Optimism),
                tempo: true,
                ..Default::default()
            };
            assert!(cfg.is_optimism());
            assert!(!cfg.is_tempo());
        }

        #[test]
        fn serde_roundtrip_optimism() {
            let original = NetworkConfigs::with_optimism();
            let json = serde_json::to_string(&original).unwrap();
            let restored: NetworkConfigs = serde_json::from_str(&json).unwrap();
            assert!(restored.is_optimism());
            assert!(!restored.is_tempo());
        }

        #[test]
        fn serde_optimism_field_deserialized() {
            let json_optimism =
                r#"{"network": "optimism", "celo": false, "bypass_prevrandao": false}"#;
            let cfg_optimism: NetworkConfigs = serde_json::from_str(json_optimism).unwrap();
            assert!(cfg_optimism.is_optimism());
        }
    }
}
