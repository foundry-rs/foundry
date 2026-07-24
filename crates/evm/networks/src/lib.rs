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
use alloy_evm::precompiles::{DynPrecompile, PrecompilesMap};
use alloy_primitives::{Address, ChainId, address, map::AddressHashMap};
use clap::Parser;
#[cfg(feature = "monad")]
use foundry_evm_hardforks::MonadHardfork;
use foundry_evm_hardforks::{FoundryHardfork, TempoHardfork};
#[cfg(not(feature = "monad"))]
type MonadHardfork = ();
#[cfg(feature = "monad")]
use monad_revm::{
    MONAD_MAX_CODE_SIZE, MONAD_MAX_INITCODE_SIZE, reserve_balance::abi::RESERVE_BALANCE_ADDRESS,
    staking::STAKING_ADDRESS,
};
use revm::precompile::{
    Precompile as RevmPrecompile,
    secp256r1::{P256VERIFY, P256VERIFY_OSAKA},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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

#[cfg(feature = "monad")]
const MONAD_PRECOMPILE_LABELS: &[(&str, Address)] =
    &[("Staking", STAKING_ADDRESS), ("ReserveBalance", RESERVE_BALANCE_ADDRESS)];

#[cfg(feature = "monad")]
const MONAD_PRECOMPILES: &[(&str, Address)] =
    &[("MonadStaking", STAKING_ADDRESS), ("MonadReserveBalance", RESERVE_BALANCE_ADDRESS)];

/// BSC secp256r1 precompile address introduced by the Haber hardfork.
const BSC_P256_ADDRESS: Address = address!("0000000000000000000000000000000000000100");

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

/// Returns whether a well-known Monad precompile address is active at `hardfork`.
#[cfg(feature = "monad")]
pub fn is_monad_precompile_active_at(address: Address, hardfork: MonadHardfork) -> bool {
    address == STAKING_ADDRESS
        || (address == RESERVE_BALANCE_ADDRESS && MonadHardfork::MonadNine.is_enabled_in(hardfork))
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
    #[cfg(feature = "monad")]
    Monad,
}

/// Runtime and initcode byte-size limits for a configured network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkContractSizeLimits {
    /// Maximum deployed runtime bytecode size.
    pub runtime: usize,
    /// Maximum initcode bytecode size.
    pub initcode: usize,
}

impl std::str::FromStr for NetworkVariant {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ethereum" => Ok(Self::Ethereum),
            #[cfg(feature = "optimism")]
            "optimism" => Ok(Self::Optimism),
            "tempo" => Ok(Self::Tempo),
            #[cfg(feature = "monad")]
            "monad" => Ok(Self::Monad),
            _ => Err(format!("unknown network variant: {s}")),
        }
    }
}

impl NetworkVariant {
    /// Returns `true` if this is the Ethereum network variant.
    pub const fn is_ethereum(&self) -> bool {
        matches!(self, Self::Ethereum)
    }

    /// Returns `true` if this is the Optimism network variant.
    #[cfg(feature = "optimism")]
    pub const fn is_optimism(&self) -> bool {
        matches!(self, Self::Optimism)
    }

    /// Returns `true` if this is the Tempo network variant.
    pub const fn is_tempo(&self) -> bool {
        matches!(self, Self::Tempo)
    }

    /// Returns `true` if this is the Monad network variant.
    #[cfg(feature = "monad")]
    pub const fn is_monad(&self) -> bool {
        matches!(self, Self::Monad)
    }

    /// Returns the network variant name.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Ethereum => "ethereum",
            #[cfg(feature = "optimism")]
            Self::Optimism => "optimism",
            Self::Tempo => "tempo",
            #[cfg(feature = "monad")]
            Self::Monad => "monad",
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
        #[cfg(feature = "monad")]
        if matches!(chain.named(), Some(NamedChain::Monad | NamedChain::MonadTestnet)) {
            return Self::Monad;
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
    #[cfg_attr(feature = "monad", arg(conflicts_with = "monad"))]
    #[serde(default)]
    pub(crate) network: Option<NetworkVariant>,
    /// Enable Celo network features.
    #[arg(help_heading = "Networks", long, conflicts_with_all = ["network", "tempo"])]
    #[cfg_attr(feature = "optimism", arg(conflicts_with = "optimism"))]
    #[cfg_attr(feature = "monad", arg(conflicts_with = "monad"))]
    celo: bool,
    /// Enable Optimism network features (deprecated: use --network optimism).
    #[cfg(feature = "optimism")]
    #[arg(long, hide = true, conflicts_with_all = ["network", "celo", "tempo"])]
    #[cfg_attr(feature = "monad", arg(conflicts_with = "monad"))]
    // Deserialize-only legacy alias: accepted in foundry.toml but never serialized — the
    // canonical form is `network = "optimism"`.
    #[serde(default)]
    pub(crate) optimism: bool,
    /// Enable Tempo network features (deprecated: use --network tempo).
    #[arg(long, hide = true, conflicts_with_all = ["network", "celo"])]
    #[cfg_attr(feature = "optimism", arg(conflicts_with = "optimism"))]
    #[cfg_attr(feature = "monad", arg(conflicts_with = "monad"))]
    // Deserialize-only legacy alias: accepted in foundry.toml but never serialized — the
    // canonical form is `network = "tempo"`.
    #[serde(default)]
    tempo: bool,
    /// Enable Monad network features (deprecated: use --network monad).
    #[cfg(feature = "monad")]
    #[arg(long, hide = true, conflicts_with_all = ["network", "celo", "tempo"])]
    #[cfg_attr(feature = "optimism", arg(conflicts_with = "optimism"))]
    // Deserialize-only legacy alias: accepted in foundry.toml but never serialized - the
    // canonical form is `network = "monad"`.
    #[serde(default)]
    monad: bool,
    /// Whether to bypass prevrandao.
    #[arg(skip)]
    #[serde(default)]
    bypass_prevrandao: bool,
}

// Custom `Serialize` impl: always emits the *resolved* network as the canonical
// `network = "..."` field, and never emits the legacy `tempo` / `optimism` / `monad` aliases.
// This avoids confusing output like `network = "monad"` next to `monad = false`, and ensures
// legacy aliases in foundry.toml round-trip as canonical network values.
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

    #[cfg(feature = "monad")]
    pub fn with_monad() -> Self {
        Self { network: Some(NetworkVariant::Monad), monad: true, ..Default::default() }
    }

    pub const fn is_tempo(&self) -> bool {
        if let Some(network) = self.resolved_network() { network.is_tempo() } else { false }
    }

    #[cfg(feature = "monad")]
    pub const fn is_monad(&self) -> bool {
        if let Some(network) = self.resolved_network() { network.is_monad() } else { false }
    }

    #[cfg(not(feature = "monad"))]
    pub const fn is_monad(&self) -> bool {
        false
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
        #[cfg(feature = "monad")]
        if self.monad {
            return Some(NetworkVariant::Monad);
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

    /// Returns contract size limits for networks that override Ethereum defaults.
    #[cfg(feature = "monad")]
    pub fn contract_size_limits(&self) -> Option<NetworkContractSizeLimits> {
        self.is_monad().then_some(NetworkContractSizeLimits {
            runtime: MONAD_MAX_CODE_SIZE,
            initcode: MONAD_MAX_INITCODE_SIZE,
        })
    }

    /// Returns contract size limits for networks that override Ethereum defaults.
    #[cfg(not(feature = "monad"))]
    pub const fn contract_size_limits(&self) -> Option<NetworkContractSizeLimits> {
        None
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
        #[cfg(feature = "monad")]
        if matches!(chain.named(), Some(NamedChain::Monad | NamedChain::MonadTestnet)) {
            return Self::with_monad();
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
            #[cfg(feature = "monad")]
            FoundryHardfork::Monad(_) => Self::with_monad(),
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

    /// Returns precompiles label for configured networks, to be used in traces.
    pub fn precompiles_label(
        self,
        tempo_hardfork: Option<TempoHardfork>,
        monad_hardfork: Option<MonadHardfork>,
    ) -> AddressHashMap<String> {
        #[cfg(not(feature = "monad"))]
        let _ = monad_hardfork;
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
        #[cfg(feature = "monad")]
        if self.is_monad() {
            labels.extend(
                MONAD_PRECOMPILE_LABELS
                    .iter()
                    .copied()
                    .filter(|(_, address)| {
                        monad_hardfork.is_none_or(|hardfork| {
                            is_monad_precompile_active_at(*address, hardfork)
                        })
                    })
                    .map(|(label, address)| (address, label.to_string())),
            );
        }
        labels
    }

    /// Returns precompiles for configured networks.
    pub fn precompiles(
        self,
        tempo_hardfork: Option<TempoHardfork>,
        monad_hardfork: Option<MonadHardfork>,
    ) -> BTreeMap<String, Address> {
        #[cfg(not(feature = "monad"))]
        let _ = monad_hardfork;
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
        #[cfg(feature = "monad")]
        if self.is_monad() {
            precompiles.extend(
                MONAD_PRECOMPILES
                    .iter()
                    .copied()
                    .filter(|(_, address)| {
                        monad_hardfork.is_none_or(|hardfork| {
                            is_monad_precompile_active_at(*address, hardfork)
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
            #[cfg(feature = "monad")]
            NetworkVariant::Monad => {
                Self { network: Some(network), monad: true, ..Default::default() }
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
    fn network_variant_predicates() {
        assert!(NetworkVariant::Ethereum.is_ethereum());
        assert!(!NetworkVariant::Ethereum.is_tempo());
        assert!(NetworkVariant::Tempo.is_tempo());
        assert!(!NetworkVariant::Tempo.is_ethereum());

        #[cfg(feature = "monad")]
        {
            assert!(!NetworkVariant::Ethereum.is_monad());
            assert!(!NetworkVariant::Tempo.is_monad());
            assert!(NetworkVariant::Monad.is_monad());
            assert!(!NetworkVariant::Monad.is_ethereum());
            assert!(!NetworkVariant::Monad.is_tempo());
        }

        #[cfg(feature = "optimism")]
        {
            assert!(NetworkVariant::Optimism.is_optimism());
            assert!(!NetworkVariant::Optimism.is_ethereum());
            assert!(!NetworkVariant::Optimism.is_tempo());

            #[cfg(feature = "monad")]
            assert!(!NetworkVariant::Optimism.is_monad());
        }

        #[cfg(all(feature = "optimism", feature = "monad"))]
        assert!(!NetworkVariant::Monad.is_optimism());
    }

    #[test]
    fn new_tempo_flag_equivalent_to_legacy() {
        let via_new = NetworkConfigs { network: Some(NetworkVariant::Tempo), ..Default::default() };
        let via_old = NetworkConfigs { tempo: true, ..Default::default() };
        assert_eq!(via_new.is_tempo(), via_old.is_tempo());
        assert_eq!(via_new.active_network_name(), via_old.active_network_name());
        assert_eq!(via_new.precompiles(None, None), via_old.precompiles(None, None));
        assert_eq!(via_new.precompiles_label(None, None), via_old.precompiles_label(None, None));
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
    fn canonical_tempo_network_reports_precompiles() {
        let cfg = NetworkConfigs { network: Some(NetworkVariant::Tempo), ..Default::default() };

        assert_eq!(
            cfg.precompiles(None, None).get("TIP20ChannelReserve"),
            Some(&TIP20_CHANNEL_RESERVE_ADDRESS)
        );
        assert!(
            !cfg.precompiles(Some(TempoHardfork::T4), None).contains_key("TIP20ChannelReserve")
        );
        assert!(!cfg.precompiles(Some(TempoHardfork::T4), None).contains_key("ReceivePolicyGuard"));
        assert!(!cfg.precompiles(Some(TempoHardfork::T2), None).contains_key("AddressRegistry"));
        assert!(!cfg.precompiles(Some(TempoHardfork::T2), None).contains_key("SignatureVerifier"));
        assert_eq!(
            cfg.precompiles(Some(TempoHardfork::T3), None).get("AddressRegistry"),
            Some(&ADDRESS_REGISTRY_ADDRESS)
        );
        assert_eq!(
            cfg.precompiles(Some(TempoHardfork::T3), None).get("SignatureVerifier"),
            Some(&SIGNATURE_VERIFIER_ADDRESS)
        );
        assert_eq!(
            cfg.precompiles_label(Some(TempoHardfork::T5), None)
                .get(&TIP20_CHANNEL_RESERVE_ADDRESS),
            Some(&"TIP20ChannelReserve".to_string())
        );
        assert!(cfg.precompiles_label(None, None).contains_key(&TIP20_CHANNEL_RESERVE_ADDRESS));
        assert!(
            !cfg.precompiles_label(Some(TempoHardfork::T5), None)
                .contains_key(&RECEIVE_POLICY_GUARD_ADDRESS)
        );
        assert!(
            cfg.precompiles_label(Some(TempoHardfork::T6), None)
                .contains_key(&RECEIVE_POLICY_GUARD_ADDRESS)
        );
    }

    #[test]
    #[cfg(feature = "monad")]
    fn canonical_monad_network_reports_hardfork_gated_precompiles() {
        let cfg = NetworkConfigs { network: Some(NetworkVariant::Monad), ..Default::default() };

        assert_eq!(
            cfg.precompiles(None, Some(MonadHardfork::MonadEight)).get("MonadStaking"),
            Some(&STAKING_ADDRESS)
        );
        assert!(
            !cfg.precompiles(None, Some(MonadHardfork::MonadEight))
                .contains_key("MonadReserveBalance")
        );
        assert_eq!(
            cfg.precompiles(None, Some(MonadHardfork::MonadNine)).get("MonadReserveBalance"),
            Some(&RESERVE_BALANCE_ADDRESS)
        );
        assert_eq!(
            cfg.precompiles_label(None, Some(MonadHardfork::MonadNine))
                .get(&RESERVE_BALANCE_ADDRESS),
            Some(&"ReserveBalance".to_string())
        );
        assert!(cfg.precompiles_label(None, None).contains_key(&RESERVE_BALANCE_ADDRESS));
        assert!(
            !cfg.precompiles_label(None, Some(MonadHardfork::MonadEight))
                .contains_key(&RESERVE_BALANCE_ADDRESS)
        );
    }

    #[test]
    fn storage_credits_precompile_activates_at_t7() {
        assert!(!is_tempo_precompile_active_at(STORAGE_CREDITS_ADDRESS, TempoHardfork::T6));
        assert!(is_tempo_precompile_active_at(STORAGE_CREDITS_ADDRESS, TempoHardfork::T7));
        assert!(TEMPO_PRECOMPILE_ADDRESSES.contains(&STORAGE_CREDITS_ADDRESS));

        // The hardfork-filtered precompile map must honor the same T7 activation.
        let cfg = NetworkConfigs { network: Some(NetworkVariant::Tempo), ..Default::default() };
        assert!(!cfg.precompiles(Some(TempoHardfork::T6), None).contains_key("StorageCredits"));
        assert!(cfg.precompiles(Some(TempoHardfork::T7), None).contains_key("StorageCredits"));
    }

    #[test]
    fn current_committee_precompile_activates_at_t8() {
        assert!(!is_tempo_precompile_active_at(CURRENT_COMMITTEE_ADDRESS, TempoHardfork::T7));
        assert!(is_tempo_precompile_active_at(CURRENT_COMMITTEE_ADDRESS, TempoHardfork::T8));
        assert!(TEMPO_PRECOMPILE_ADDRESSES.contains(&CURRENT_COMMITTEE_ADDRESS));

        let cfg = NetworkConfigs { network: Some(NetworkVariant::Tempo), ..Default::default() };
        assert!(!cfg.precompiles(Some(TempoHardfork::T7), None).contains_key("CurrentCommittee"));
        assert!(cfg.precompiles(Some(TempoHardfork::T8), None).contains_key("CurrentCommittee"));
    }

    // --- resolved() / active_network_name ---

    #[test]
    fn active_network_name_tempo() {
        let cfg = NetworkConfigs::with_tempo();
        assert_eq!(cfg.active_network_name(), Some("tempo"));
    }

    #[test]
    #[cfg(feature = "monad")]
    fn active_network_name_monad() {
        let cfg = NetworkConfigs::with_monad();
        assert_eq!(cfg.active_network_name(), Some("monad"));
        assert!(cfg.is_monad());
    }

    #[test]
    #[cfg(feature = "monad")]
    fn contract_size_limits_monad() {
        let limits = NetworkConfigs::with_monad().contract_size_limits().unwrap();
        assert_eq!(limits.runtime, MONAD_MAX_CODE_SIZE);
        assert_eq!(limits.initcode, MONAD_MAX_INITCODE_SIZE);
        assert!(NetworkConfigs::default().contract_size_limits().is_none());
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
    #[cfg(feature = "monad")]
    fn serde_roundtrip_monad() {
        let original = NetworkConfigs::with_monad();
        let json = serde_json::to_string(&original).unwrap();
        let restored: NetworkConfigs = serde_json::from_str(&json).unwrap();
        assert!(restored.is_monad());
        assert!(!restored.is_tempo());
    }

    #[test]
    #[cfg(feature = "monad")]
    fn serde_legacy_monad_bool_deserialized() {
        let json = r#"{"monad": true, "celo": false, "bypass_prevrandao": false}"#;
        let cfg: NetworkConfigs = serde_json::from_str(json).unwrap();
        assert!(cfg.is_monad());
    }

    #[test]
    fn serde_serializes_legacy_alias_as_canonical_network() {
        #[cfg_attr(not(feature = "monad"), allow(unused_mut))]
        let mut cases = vec![(NetworkConfigs { tempo: true, ..Default::default() }, "tempo")];
        #[cfg(feature = "monad")]
        cases.push((NetworkConfigs { monad: true, ..Default::default() }, "monad"));

        for (cfg, expected) in cases {
            let json = serde_json::to_value(cfg).unwrap();
            assert_eq!(json["network"], serde_json::json!(expected));
            assert!(json.get("tempo").is_none(), "legacy `tempo` key should not be serialized");
            assert!(json.get("monad").is_none(), "legacy `monad` key should not be serialized");
        }
    }

    #[test]
    fn serde_new_network_field_deserialized() {
        let json_tempo = r#"{"network": "tempo", "celo": false, "bypass_prevrandao": false}"#;
        let cfg_tempo: NetworkConfigs = serde_json::from_str(json_tempo).unwrap();
        assert!(cfg_tempo.is_tempo());

        #[cfg(feature = "monad")]
        {
            let json_monad = r#"{"network": "monad", "celo": false, "bypass_prevrandao": false}"#;
            let cfg_monad: NetworkConfigs = serde_json::from_str(json_monad).unwrap();
            assert!(cfg_monad.is_monad());
        }
    }

    #[test]
    #[cfg(feature = "monad")]
    fn chain_id_detects_monad_network() {
        assert_eq!(NetworkVariant::from(143), NetworkVariant::Monad);
        assert_eq!(NetworkVariant::from(10143), NetworkVariant::Monad);

        assert!(NetworkConfigs::default().with_chain_id(143).is_monad());
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
