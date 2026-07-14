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
use alloy_evm::precompiles::{DynPrecompile, PrecompileInput, PrecompilesMap};
use alloy_primitives::{Address, Bytes, ChainId, address, map::AddressHashMap};
use clap::Parser;
use foundry_evm_hardforks::{FoundryHardfork, TempoHardfork};
use revm::precompile::{
    Precompile as RevmPrecompile, PrecompileOutput,
    secp256r1::{P256VERIFY, P256VERIFY_OSAKA},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};
use tempo_contracts::precompiles::{
    ACCOUNT_KEYCHAIN_ADDRESS, ADDRESS_REGISTRY_ADDRESS, CURRENT_COMMITTEE_ADDRESS,
    NONCE_PRECOMPILE_ADDRESS, RECEIVE_POLICY_GUARD_ADDRESS, SIGNATURE_VERIFIER_ADDRESS,
    STABLECOIN_DEX_ADDRESS, STORAGE_CREDITS_ADDRESS, TIP_FEE_MANAGER_ADDRESS,
    TIP20_CHANNEL_RESERVE_ADDRESS, TIP20_FACTORY_ADDRESS, TIP403_REGISTRY_ADDRESS,
    VALIDATOR_CONFIG_ADDRESS, VALIDATOR_CONFIG_V2_ADDRESS,
};

pub mod arbitrum;
pub mod celo;

#[cfg(feature = "optimism")]
mod optimism;

const TEMPO_PRECOMPILES: &[(&str, Address)] = &[
    ("Nonce", NONCE_PRECOMPILE_ADDRESS),
    ("StablecoinDex", STABLECOIN_DEX_ADDRESS),
    ("TIP20Factory", TIP20_FACTORY_ADDRESS),
    ("TIP403Registry", TIP403_REGISTRY_ADDRESS),
    ("FeeManager", TIP_FEE_MANAGER_ADDRESS),
    ("ValidatorConfig", VALIDATOR_CONFIG_ADDRESS),
    ("ValidatorConfigV2", VALIDATOR_CONFIG_V2_ADDRESS),
    ("AccountKeychain", ACCOUNT_KEYCHAIN_ADDRESS),
    ("SignatureVerifier", SIGNATURE_VERIFIER_ADDRESS),
    ("AddressRegistry", ADDRESS_REGISTRY_ADDRESS),
    ("TIP20ChannelReserve", TIP20_CHANNEL_RESERVE_ADDRESS),
    ("ReceivePolicyGuard", RECEIVE_POLICY_GUARD_ADDRESS),
    ("StorageCredits", STORAGE_CREDITS_ADDRESS),
    ("CurrentCommittee", CURRENT_COMMITTEE_ADDRESS),
];

/// BSC secp256r1 precompile address introduced by the Haber hardfork.
pub const BSC_P256_ADDRESS: Address = address!("0000000000000000000000000000000000000100");

const BSC_MAINNET_CHAIN_ID: u64 = 56;
const BSC_TESTNET_CHAIN_ID: u64 = 97;
const BSC_MAINNET_HABER_TIMESTAMP: u64 = 1_718_863_500;
const BSC_TESTNET_HABER_TIMESTAMP: u64 = 1_716_962_820;
const BSC_MAINNET_OSAKA_TIMESTAMP: u64 = 1_777_343_400;
const BSC_TESTNET_OSAKA_TIMESTAMP: u64 = 1_774_319_400;

/// Returns the BSC P256 precompile for the given timestamp. The outer option distinguishes BSC
/// chains from unrelated chains, while the inner option disables P256 before Haber.
const fn bsc_p256_precompile(chain_id: ChainId, timestamp: u64) -> Option<Option<RevmPrecompile>> {
    let (haber_timestamp, osaka_timestamp) = match chain_id {
        BSC_MAINNET_CHAIN_ID => (BSC_MAINNET_HABER_TIMESTAMP, BSC_MAINNET_OSAKA_TIMESTAMP),
        BSC_TESTNET_CHAIN_ID => (BSC_TESTNET_HABER_TIMESTAMP, BSC_TESTNET_OSAKA_TIMESTAMP),
        _ => return None,
    };

    if timestamp < haber_timestamp {
        Some(None)
    } else if timestamp < osaka_timestamp {
        Some(Some(P256VERIFY))
    } else {
        Some(Some(P256VERIFY_OSAKA))
    }
}

/// Shared chain context used by Foundry EVMs whose active fork can change during execution.
#[derive(Clone, Debug, Default)]
pub struct ChainPrecompileState(Arc<RwLock<(ChainId, u64)>>);

impl ChainPrecompileState {
    /// Returns an independent state initialized from the current chain context.
    pub fn fork(&self) -> Self {
        Self(Arc::new(RwLock::new(self.get())))
    }

    /// Updates the active chain context.
    pub fn set(&self, chain_id: ChainId, timestamp: u64) {
        *self.0.write().unwrap_or_else(|err| err.into_inner()) = (chain_id, timestamp);
    }

    /// Returns the active chain context.
    pub fn get(&self) -> (ChainId, u64) {
        *self.0.read().unwrap_or_else(|err| err.into_inner())
    }
}

fn dynamic_p256_precompile(
    has_fallback: bool,
    state: Option<ChainPrecompileState>,
) -> DynPrecompile {
    DynPrecompile::new_stateful(P256VERIFY.id().clone(), move |input: PrecompileInput<'_>| {
        let chain_id = input.internals.chain_id();
        let timestamp = input.internals.block_timestamp().saturating_to();
        match bsc_p256_precompile(chain_id, timestamp) {
            Some(Some(precompile)) => precompile.execute(input.data, input.gas, input.reservoir),
            Some(None) => Ok(PrecompileOutput::new(0, Bytes::new(), input.reservoir)),
            None => {
                if has_fallback {
                    P256VERIFY_OSAKA.execute(input.data, input.gas, input.reservoir)
                } else if let Some(state) = &state {
                    let (chain_id, timestamp) = state.get();
                    if let Some(Some(precompile)) = bsc_p256_precompile(chain_id, timestamp) {
                        return precompile.execute(input.data, input.gas, input.reservoir);
                    }
                    Ok(PrecompileOutput::new(0, Bytes::new(), input.reservoir))
                } else {
                    Ok(PrecompileOutput::new(0, Bytes::new(), input.reservoir))
                }
            }
        }
    })
}

/// All well-known Tempo precompile addresses.
pub const TEMPO_PRECOMPILE_ADDRESSES: &[Address] = &[
    NONCE_PRECOMPILE_ADDRESS,
    STABLECOIN_DEX_ADDRESS,
    TIP20_FACTORY_ADDRESS,
    TIP403_REGISTRY_ADDRESS,
    TIP_FEE_MANAGER_ADDRESS,
    VALIDATOR_CONFIG_ADDRESS,
    VALIDATOR_CONFIG_V2_ADDRESS,
    ACCOUNT_KEYCHAIN_ADDRESS,
    SIGNATURE_VERIFIER_ADDRESS,
    ADDRESS_REGISTRY_ADDRESS,
    TIP20_CHANNEL_RESERVE_ADDRESS,
    RECEIVE_POLICY_GUARD_ADDRESS,
    STORAGE_CREDITS_ADDRESS,
    CURRENT_COMMITTEE_ADDRESS,
];

/// Returns whether a well-known Tempo precompile address is active at `hardfork`.
pub fn is_tempo_precompile_active_at(address: Address, hardfork: TempoHardfork) -> bool {
    if address == CURRENT_COMMITTEE_ADDRESS {
        hardfork.is_t8()
    } else if address == TIP20_CHANNEL_RESERVE_ADDRESS {
        hardfork.is_t5()
    } else if address == RECEIVE_POLICY_GUARD_ADDRESS {
        hardfork.is_t6()
    } else if address == STORAGE_CREDITS_ADDRESS {
        hardfork.is_t7()
    } else if address == ADDRESS_REGISTRY_ADDRESS || address == SIGNATURE_VERIFIER_ADDRESS {
        hardfork.is_t3()
    } else {
        true
    }
}

/// Returns the well-known Tempo precompile addresses active at `hardfork`.
pub fn active_tempo_precompile_addresses(hardfork: TempoHardfork) -> impl Iterator<Item = Address> {
    TEMPO_PRECOMPILE_ADDRESSES
        .iter()
        .copied()
        .filter(move |&address| is_tempo_precompile_active_at(address, hardfork))
}

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
    pub const fn resolved_network(&self) -> Option<NetworkVariant> {
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
    #[cfg(feature = "optimism")]
    pub fn base_fee_params(&self, timestamp: u64) -> BaseFeeParams {
        if self.is_optimism() {
            return self.op_base_fee_params(timestamp);
        }
        BaseFeeParams::ethereum()
    }

    /// Returns the base fee parameters for the configured network.
    #[cfg(not(feature = "optimism"))]
    pub const fn base_fee_params(&self, timestamp: u64) -> BaseFeeParams {
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

    /// Injects chain-specific precompiles active at the given timestamp.
    pub fn inject_chain_precompiles(
        self,
        precompiles: &mut PrecompilesMap,
        chain_id: ChainId,
        timestamp: u64,
    ) {
        let Some(p256verify) = bsc_p256_precompile(chain_id, timestamp) else { return };
        precompiles.apply_precompile(&BSC_P256_ADDRESS, move |_| {
            p256verify.map(|p256verify| {
                DynPrecompile::new(p256verify.id().clone(), move |input| {
                    p256verify.execute(input.data, input.gas, input.reservoir)
                })
            })
        });
    }

    /// Configures chain-specific precompiles for an EVM that can switch forks while running.
    pub fn inject_dynamic_chain_precompiles(
        self,
        precompiles: &mut PrecompilesMap,
        state: ChainPrecompileState,
    ) {
        let (chain_id, timestamp) = state.get();
        let mut has_fallback = false;
        precompiles.apply_precompile(&BSC_P256_ADDRESS, |existing| {
            has_fallback = existing.is_some();
            None
        });

        let p256verify = bsc_p256_precompile(chain_id, timestamp);
        let install_static = match &p256verify {
            Some(Some(_)) => true,
            None => has_fallback,
            Some(None) => false,
        };
        if install_static {
            let precompile = dynamic_p256_precompile(has_fallback, Some(state));
            precompiles.apply_precompile(&BSC_P256_ADDRESS, |_| Some(precompile));
        } else {
            precompiles.set_precompile_lookup(move |address: &Address| {
                if address != &BSC_P256_ADDRESS {
                    return None;
                }

                let (chain_id, timestamp) = state.get();
                match bsc_p256_precompile(chain_id, timestamp) {
                    Some(Some(precompile)) => {
                        Some(DynPrecompile::new(precompile.id().clone(), move |input| {
                            precompile.execute(input.data, input.gas, input.reservoir)
                        }))
                    }
                    Some(None) => None,
                    None if has_fallback => {
                        Some(DynPrecompile::new(P256VERIFY_OSAKA.id().clone(), move |input| {
                            P256VERIFY_OSAKA.execute(input.data, input.gas, input.reservoir)
                        }))
                    }
                    None => None,
                }
            });
        }
    }

    /// Applies chain-specific precompile activation to a warm-address set.
    pub fn inject_chain_precompile_addresses(
        self,
        addresses: &mut alloy_primitives::map::AddressSet,
        chain_id: ChainId,
        timestamp: u64,
    ) {
        match bsc_p256_precompile(chain_id, timestamp) {
            Some(Some(_)) => {
                addresses.insert(BSC_P256_ADDRESS);
            }
            Some(None) => {
                addresses.remove(&BSC_P256_ADDRESS);
            }
            None => {}
        }
    }

    /// Returns precompiles label for configured networks, to be used in traces.
    pub fn precompiles_label(
        self,
        tempo_hardfork: Option<TempoHardfork>,
    ) -> AddressHashMap<String> {
        let mut labels = AddressHashMap::default();
        if self.celo {
            labels.insert(CELO_TRANSFER_ADDRESS, CELO_TRANSFER_LABEL.to_string());
        }
        if self.is_tempo() {
            labels.extend(
                TEMPO_PRECOMPILES
                    .iter()
                    .copied()
                    .filter(|(_, address)| {
                        tempo_hardfork.is_none_or(|hardfork| {
                            is_tempo_precompile_active_at(*address, hardfork)
                        })
                    })
                    .map(|(label, address)| (address, label.to_string())),
            );
        }
        labels
    }

    /// Returns precompiles for configured networks.
    pub fn precompiles(self, tempo_hardfork: Option<TempoHardfork>) -> BTreeMap<String, Address> {
        let mut precompiles = BTreeMap::new();
        if self.celo {
            precompiles
                .insert(PRECOMPILE_ID_CELO_TRANSFER.name().to_string(), CELO_TRANSFER_ADDRESS);
        }
        if self.is_tempo() {
            precompiles.extend(
                TEMPO_PRECOMPILES
                    .iter()
                    .copied()
                    .filter(|(_, address)| {
                        tempo_hardfork.is_none_or(|hardfork| {
                            is_tempo_precompile_active_at(*address, hardfork)
                        })
                    })
                    .map(|(label, address)| (label.to_string(), address)),
            );
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
    use revm::precompile::{
        Precompiles,
        secp256r1::{P256VERIFY_BASE_GAS_FEE, P256VERIFY_BASE_GAS_FEE_OSAKA},
    };

    // --- Equivalence: new flag == legacy flag ---

    #[test]
    fn new_tempo_flag_equivalent_to_legacy() {
        let via_new = NetworkConfigs { network: Some(NetworkVariant::Tempo), ..Default::default() };
        let via_old = NetworkConfigs { tempo: true, ..Default::default() };
        assert_eq!(via_new.is_tempo(), via_old.is_tempo());
        assert_eq!(via_new.active_network_name(), via_old.active_network_name());
        assert_eq!(via_new.precompiles(None), via_old.precompiles(None));
        assert_eq!(via_new.precompiles_label(None), via_old.precompiles_label(None));
    }

    fn bsc_p256_gas_used(chain_id: ChainId, timestamp: u64) -> Option<u64> {
        bsc_p256_precompile(chain_id, timestamp)
            .flatten()
            .map(|precompile| precompile.execute(&[], u64::MAX, 0).unwrap().gas_used)
    }

    fn assert_bsc_p256_boundaries(chain_id: ChainId, haber_timestamp: u64, osaka_timestamp: u64) {
        assert!(matches!(bsc_p256_precompile(chain_id, haber_timestamp - 1), Some(None)));
        assert_eq!(bsc_p256_gas_used(chain_id, haber_timestamp), Some(P256VERIFY_BASE_GAS_FEE));
        assert_eq!(bsc_p256_gas_used(chain_id, osaka_timestamp - 1), Some(P256VERIFY_BASE_GAS_FEE));
        assert_eq!(
            bsc_p256_gas_used(chain_id, osaka_timestamp),
            Some(P256VERIFY_BASE_GAS_FEE_OSAKA)
        );
    }

    #[test]
    fn selects_bsc_p256_at_mainnet_boundaries() {
        assert_bsc_p256_boundaries(
            BSC_MAINNET_CHAIN_ID,
            BSC_MAINNET_HABER_TIMESTAMP,
            BSC_MAINNET_OSAKA_TIMESTAMP,
        );
    }

    #[test]
    fn selects_bsc_p256_at_testnet_boundaries() {
        assert_bsc_p256_boundaries(
            BSC_TESTNET_CHAIN_ID,
            BSC_TESTNET_HABER_TIMESTAMP,
            BSC_TESTNET_OSAKA_TIMESTAMP,
        );
    }

    #[test]
    fn removes_bsc_p256_before_haber() {
        let mut precompiles = PrecompilesMap::from_static(Precompiles::osaka());
        assert!(precompiles.get(&BSC_P256_ADDRESS).is_some());
        NetworkConfigs::default().inject_chain_precompiles(
            &mut precompiles,
            BSC_MAINNET_CHAIN_ID,
            BSC_MAINNET_HABER_TIMESTAMP - 1,
        );
        assert!(precompiles.get(&BSC_P256_ADDRESS).is_none());
    }

    #[test]
    fn dynamic_bsc_p256_tracks_pre_haber_to_haber_roll() {
        let state = ChainPrecompileState::default();
        state.set(BSC_MAINNET_CHAIN_ID, BSC_MAINNET_HABER_TIMESTAMP - 1);
        let mut precompiles = PrecompilesMap::from_static(Precompiles::osaka());
        NetworkConfigs::default().inject_dynamic_chain_precompiles(&mut precompiles, state.clone());
        assert!(precompiles.get(&BSC_P256_ADDRESS).is_none());

        state.set(BSC_MAINNET_CHAIN_ID, BSC_MAINNET_HABER_TIMESTAMP);
        assert!(precompiles.get(&BSC_P256_ADDRESS).is_some());

        state.set(BSC_MAINNET_CHAIN_ID, BSC_MAINNET_HABER_TIMESTAMP - 1);
        assert!(precompiles.get(&BSC_P256_ADDRESS).is_none());
    }

    #[test]
    fn bsc_p256_warm_address_tracks_haber_boundary() {
        let mut addresses = Precompiles::osaka().addresses_set().clone();
        assert!(addresses.contains(&BSC_P256_ADDRESS));

        NetworkConfigs::default().inject_chain_precompile_addresses(
            &mut addresses,
            BSC_MAINNET_CHAIN_ID,
            BSC_MAINNET_HABER_TIMESTAMP - 1,
        );
        assert!(!addresses.contains(&BSC_P256_ADDRESS));

        NetworkConfigs::default().inject_chain_precompile_addresses(
            &mut addresses,
            BSC_MAINNET_CHAIN_ID,
            BSC_MAINNET_HABER_TIMESTAMP,
        );
        assert!(addresses.contains(&BSC_P256_ADDRESS));
    }

    #[test]
    fn canonical_tempo_network_reports_precompiles() {
        let cfg = NetworkConfigs { network: Some(NetworkVariant::Tempo), ..Default::default() };

        assert_eq!(
            cfg.precompiles(None).get("TIP20ChannelReserve"),
            Some(&TIP20_CHANNEL_RESERVE_ADDRESS)
        );
        assert!(!cfg.precompiles(Some(TempoHardfork::T4)).contains_key("TIP20ChannelReserve"));
        assert!(!cfg.precompiles(Some(TempoHardfork::T4)).contains_key("ReceivePolicyGuard"));
        assert!(!cfg.precompiles(Some(TempoHardfork::T2)).contains_key("AddressRegistry"));
        assert!(!cfg.precompiles(Some(TempoHardfork::T2)).contains_key("SignatureVerifier"));
        assert_eq!(
            cfg.precompiles(Some(TempoHardfork::T3)).get("AddressRegistry"),
            Some(&ADDRESS_REGISTRY_ADDRESS)
        );
        assert_eq!(
            cfg.precompiles(Some(TempoHardfork::T3)).get("SignatureVerifier"),
            Some(&SIGNATURE_VERIFIER_ADDRESS)
        );
        assert_eq!(
            cfg.precompiles_label(Some(TempoHardfork::T5)).get(&TIP20_CHANNEL_RESERVE_ADDRESS),
            Some(&"TIP20ChannelReserve".to_string())
        );
        assert!(cfg.precompiles_label(None).contains_key(&TIP20_CHANNEL_RESERVE_ADDRESS));
        assert!(
            !cfg.precompiles_label(Some(TempoHardfork::T5))
                .contains_key(&RECEIVE_POLICY_GUARD_ADDRESS)
        );
        assert!(
            cfg.precompiles_label(Some(TempoHardfork::T6))
                .contains_key(&RECEIVE_POLICY_GUARD_ADDRESS)
        );
    }

    #[test]
    fn storage_credits_precompile_activates_at_t7() {
        assert!(!is_tempo_precompile_active_at(STORAGE_CREDITS_ADDRESS, TempoHardfork::T6));
        assert!(is_tempo_precompile_active_at(STORAGE_CREDITS_ADDRESS, TempoHardfork::T7));
        assert!(TEMPO_PRECOMPILE_ADDRESSES.contains(&STORAGE_CREDITS_ADDRESS));

        // The hardfork-filtered precompile map must honor the same T7 activation.
        let cfg = NetworkConfigs { network: Some(NetworkVariant::Tempo), ..Default::default() };
        assert!(!cfg.precompiles(Some(TempoHardfork::T6)).contains_key("StorageCredits"));
        assert!(cfg.precompiles(Some(TempoHardfork::T7)).contains_key("StorageCredits"));
    }

    #[test]
    fn current_committee_precompile_activates_at_t8() {
        assert!(!is_tempo_precompile_active_at(CURRENT_COMMITTEE_ADDRESS, TempoHardfork::T7));
        assert!(is_tempo_precompile_active_at(CURRENT_COMMITTEE_ADDRESS, TempoHardfork::T8));
        assert!(TEMPO_PRECOMPILE_ADDRESSES.contains(&CURRENT_COMMITTEE_ADDRESS));

        let cfg = NetworkConfigs { network: Some(NetworkVariant::Tempo), ..Default::default() };
        assert!(!cfg.precompiles(Some(TempoHardfork::T7)).contains_key("CurrentCommittee"));
        assert!(cfg.precompiles(Some(TempoHardfork::T8)).contains_key("CurrentCommittee"));
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
