//! # foundry-evm-networks
//!
//! Runtime selection and shared configuration for Foundry's EVM network families.
//!
//! [`NetworkConfigs`] describes the active execution profile selected by configuration, CLI flags,
//! hardforks, or fork endpoint discovery. Cargo features only determine which optional
//! [`NetworkVariant`] values are compiled into a binary; they do not select a network at runtime.
//!
//! Concrete Alloy network and EVM factory types are associated by `FoundryEvmNetwork` in
//! `foundry-evm-core`. See the [custom EVM integration guide] for the cross-crate ownership and
//! state-lifecycle contract.
//!
//! [custom EVM integration guide]: https://github.com/foundry-rs/foundry/blob/master/docs/dev/networks.md

use crate::celo::transfer::{
    CELO_TRANSFER_ADDRESS, CELO_TRANSFER_LABEL, PRECOMPILE_ID_CELO_TRANSFER,
};
use alloy_chains::{
    Chain, NamedChain,
    NamedChain::{Chiado, Gnosis, Moonbase, Moonbeam, MoonbeamDev, Moonriver, Rsk, RskTestnet},
};
use alloy_eips::{eip1559::BaseFeeParams, eip7840::BlobParams};
use alloy_evm::precompiles::{DynPrecompile, PrecompilesMap};
use alloy_primitives::{Address, ChainId, address, map::AddressHashMap};
#[cfg(feature = "base")]
use base_common_precompiles::{
    ActivationRegistryStorage, B20FactoryStorage, NonceManagerStorage, PolicyRegistryStorage,
    TxContextStorage,
};
use clap::Parser;
#[cfg(feature = "base")]
use foundry_evm_hardforks::{BaseSpecId, BaseUpgrade};
#[cfg(feature = "monad")]
type MonadHardfork = foundry_evm_hardforks::MonadHardfork;
#[cfg(feature = "optimism")]
use foundry_evm_hardforks::OpHardfork;
use foundry_evm_hardforks::{
    EthereumHardfork, ExecutionSpec, FoundryHardfork, TempoHardfork, latest_active_tempo_hardfork,
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

/// The Monad cheatcode handler address.
pub const MONAD_CHEATCODE_ADDRESS: Address = address!("0xc0FFeeCD43A10e1C2b0De63c6CDCFe5B7d0e0CEA");

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
const MONAD_PRECOMPILE_LABELS: &[(&str, Address)] = &[
    ("Staking", monad_revm::staking::STAKING_ADDRESS),
    ("ReserveBalance", monad_revm::reserve_balance::abi::RESERVE_BALANCE_ADDRESS),
];

#[cfg(feature = "monad")]
const MONAD_PRECOMPILES: &[(&str, Address)] = &[
    ("MonadStaking", monad_revm::staking::STAKING_ADDRESS),
    ("MonadReserveBalance", monad_revm::reserve_balance::abi::RESERVE_BALANCE_ADDRESS),
];

#[cfg(feature = "base")]
const BASE_PRECOMPILES: &[(&str, Address)] = &[
    ("B20Factory", B20FactoryStorage::ADDRESS),
    ("ActivationRegistry", ActivationRegistryStorage::ADDRESS),
    ("PolicyRegistry", PolicyRegistryStorage::ADDRESS),
    ("TxContext", TxContextStorage::ADDRESS),
    ("NonceManager", NonceManagerStorage::ADDRESS),
];

/// All fixed Base precompile addresses.
#[cfg(feature = "base")]
pub const BASE_PRECOMPILE_ADDRESSES: &[Address] = &[
    B20FactoryStorage::ADDRESS,
    ActivationRegistryStorage::ADDRESS,
    PolicyRegistryStorage::ADDRESS,
    TxContextStorage::ADDRESS,
    NonceManagerStorage::ADDRESS,
];

/// Fixed Base precompiles that expose at least one function returning no data.
///
/// Solidity guards high-level calls to such functions with an `extcodesize` check, which a
/// code-less account fails in the caller, so these must carry code. Base mainnet plants a one-byte
/// sentinel on exactly these two. The factory, nonce manager, and transaction context return data
/// from every function and are code-less on chain, so stubbing them would diverge — a contract
/// guarding calls with an `isContract` probe would pass locally and revert on Base.
///
/// The nonce manager separately receives a stub at Cobalt from Base's own
/// `ensure_eip8130_system_accounts` transition, for EIP-161 state clearing rather than for
/// `extcodesize`. That transition owns it; this list must not.
#[cfg(feature = "base")]
pub const BASE_CODE_SENTINEL_ADDRESSES: &[Address] =
    &[ActivationRegistryStorage::ADDRESS, PolicyRegistryStorage::ADDRESS];

/// BSC secp256r1 precompile address introduced by the Haber hardfork.
const BSC_P256_ADDRESS: Address = address!("0000000000000000000000000000000000000100");

const BSC_MAINNET_CHAIN_ID: u64 = 56;
const BSC_TESTNET_CHAIN_ID: u64 = 97;
const BSC_MAINNET_HABER_TIMESTAMP: u64 = 1_718_863_500;
const BSC_TESTNET_HABER_TIMESTAMP: u64 = 1_716_962_820;
const BSC_MAINNET_OSAKA_TIMESTAMP: u64 = 1_777_343_400;
const BSC_TESTNET_OSAKA_TIMESTAMP: u64 = 1_774_319_400;

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
    #[cfg(feature = "base")]
    Base,
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
            #[cfg(feature = "base")]
            "base" => Ok(Self::Base),
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
    /// Returns the network family identified by a known chain ID.
    ///
    /// Unknown chain IDs return `None` so callers can consult endpoint metadata instead of
    /// assuming Ethereum. If a known chain belongs to a feature-gated family that is not enabled,
    /// this returns an error rather than silently selecting a different EVM.
    pub fn from_known_chain_id(chain_id: ChainId) -> Result<Option<Self>, String> {
        let chain = Chain::from_id(chain_id);
        if chain.is_tempo() {
            return Ok(Some(Self::Tempo));
        }
        if matches!(chain.named(), Some(NamedChain::Celo | NamedChain::CeloSepolia)) {
            return Ok(Some(Self::Ethereum));
        }
        // Only claim Base when the feature is on. `is_optimism()` below already covers Base chain
        // IDs, and that is what shipped binaries resolve them to today, so erroring here would
        // regress builds that never asked for Base. Monad errors instead because no shipped EVM
        // approximates it.
        #[cfg(feature = "base")]
        if matches!(chain.named(), Some(NamedChain::Base | NamedChain::BaseSepolia)) {
            return Ok(Some(Self::Base));
        }
        if matches!(chain.named(), Some(NamedChain::Monad | NamedChain::MonadTestnet)) {
            #[cfg(feature = "monad")]
            return Ok(Some(Self::Monad));
            #[cfg(not(feature = "monad"))]
            return Err("network family `monad` is not enabled in this build".to_string());
        }
        if chain.is_optimism() {
            #[cfg(feature = "optimism")]
            return Ok(Some(Self::Optimism));
            #[cfg(not(feature = "optimism"))]
            return Err("network family `optimism` is not enabled in this build".to_string());
        }
        Ok(chain.named().map(|_| Self::Ethereum))
    }

    /// Parses an explicit network family reported by `anvil_nodeInfo`.
    pub fn from_node_info_name(network: &str) -> Result<Self, String> {
        match network {
            "ethereum" => Ok(Self::Ethereum),
            #[cfg(feature = "base")]
            "base" => Ok(Self::Base),
            #[cfg(not(feature = "base"))]
            "base" => Err("network family `base` is not enabled in this build".to_string()),
            #[cfg(feature = "optimism")]
            "optimism" => Ok(Self::Optimism),
            #[cfg(not(feature = "optimism"))]
            "optimism" => Err("network family `optimism` is not enabled in this build".to_string()),
            "tempo" => Ok(Self::Tempo),
            #[cfg(feature = "monad")]
            "monad" => Ok(Self::Monad),
            #[cfg(not(feature = "monad"))]
            "monad" => Err("network family `monad` is not enabled in this build".to_string()),
            network => {
                Err(format!("unsupported network family `{network}` reported by fork endpoint"))
            }
        }
    }

    /// Resolves an RPC endpoint's network family.
    ///
    /// The outer `node_info_network` option distinguishes an unavailable `anvil_nodeInfo` method
    /// from a successful legacy response that omitted the network. An explicit name is
    /// authoritative even when the endpoint exposes a well-known execution chain ID. A legacy
    /// omission falls back to the known chain ID, or Ethereum for a custom chain.
    pub fn from_rpc_identity(
        chain_id: ChainId,
        node_info_network: Option<Option<&str>>,
    ) -> Result<Option<Self>, String> {
        Self::from_rpc_identity_with_fallback(chain_id, node_info_network, None)
    }

    /// Resolves an RPC endpoint's network family with a caller-selected unknown-chain fallback.
    ///
    /// Explicit endpoint metadata is authoritative. Legacy `anvil_nodeInfo` responses that omit
    /// the family first use a known chain ID, then the supplied fallback, and finally Ethereum to
    /// preserve historical behavior. Endpoints without `anvil_nodeInfo` use only a known chain ID
    /// or the supplied fallback.
    pub fn from_rpc_identity_with_fallback(
        chain_id: ChainId,
        node_info_network: Option<Option<&str>>,
        unknown_fallback: Option<Self>,
    ) -> Result<Option<Self>, String> {
        match node_info_network {
            Some(Some(network)) => Self::from_node_info_name(network).map(Some),
            Some(None) => Ok(Self::from_known_chain_id(chain_id)?
                .or(unknown_fallback)
                .or(Some(Self::Ethereum))),
            None => Ok(Self::from_known_chain_id(chain_id)?.or(unknown_fallback)),
        }
    }

    /// Parses a hardfork name reported by an RPC endpoint in this network's namespace.
    pub fn parse_hardfork(self, hardfork: &str) -> Result<FoundryHardfork, String> {
        format!("{}:{hardfork}", self.name()).parse()
    }

    /// Returns the active hardfork for this network at the given chain and timestamp.
    ///
    /// Unknown chain IDs fall back to the network's default hardfork. The selected network owns
    /// the lookup so an explicit network choice is not overridden by the chain ID's family.
    pub fn hardfork_at(self, chain_id: ChainId, timestamp: u64) -> FoundryHardfork {
        match self {
            Self::Ethereum => {
                EthereumHardfork::from_chain_and_timestamp(Chain::from_id(chain_id), timestamp)
                    .unwrap_or_default()
                    .into()
            }
            Self::Tempo => TempoHardfork::from_chain_and_timestamp(chain_id, timestamp)
                .unwrap_or_else(latest_active_tempo_hardfork)
                .into(),
            #[cfg(feature = "optimism")]
            Self::Optimism => {
                OpHardfork::from_chain_and_timestamp(Chain::from_id(chain_id), timestamp)
                    .unwrap_or_default()
                    .into()
            }
            #[cfg(feature = "monad")]
            Self::Monad => MonadHardfork::from_chain_and_timestamp(chain_id, timestamp)
                .unwrap_or_default()
                .into(),
            #[cfg(feature = "base")]
            Self::Base => BaseUpgrade::from_chain_and_timestamp(chain_id, timestamp)
                .unwrap_or_default()
                .into(),
        }
    }

    /// Returns `true` if this is the Ethereum network variant.
    pub const fn is_ethereum(&self) -> bool {
        matches!(self, Self::Ethereum)
    }

    /// Returns `true` if this is the Base network variant.
    #[cfg(feature = "base")]
    pub const fn is_base(&self) -> bool {
        matches!(self, Self::Base)
    }

    /// Returns `true` if this is the Optimism network variant.
    pub const fn is_optimism(&self) -> bool {
        #[cfg(feature = "optimism")]
        {
            matches!(self, Self::Optimism)
        }
        #[cfg(not(feature = "optimism"))]
        {
            false
        }
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

    /// Returns `false` when Monad support is not compiled in.
    #[cfg(not(feature = "monad"))]
    pub const fn is_monad(&self) -> bool {
        false
    }

    /// Returns the network variant name.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Ethereum => "ethereum",
            #[cfg(feature = "base")]
            Self::Base => "base",
            #[cfg(feature = "optimism")]
            Self::Optimism => "optimism",
            Self::Tempo => "tempo",
            #[cfg(feature = "monad")]
            Self::Monad => "monad",
        }
    }

    /// Returns the hardfork namespace used by this network family.
    pub const fn hardfork_namespace(&self) -> Option<&'static str> {
        match self {
            Self::Ethereum => None,
            #[cfg(feature = "base")]
            Self::Base => Some("base"),
            #[cfg(feature = "optimism")]
            Self::Optimism => Some("optimism"),
            Self::Tempo => Some("tempo"),
            #[cfg(feature = "monad")]
            Self::Monad => Some("monad"),
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
        Self::from_known_chain_id(chain_id).ok().flatten().unwrap_or(Self::Ethereum)
    }
}

#[derive(Clone, Debug, Default, Parser, Deserialize, Copy, PartialEq, Eq, Hash)]
pub struct NetworkConfigs {
    /// Enable a specific network family.
    #[arg(help_heading = "Networks", long, short, num_args = 1, value_name = "NETWORK", value_enum, conflicts_with_all = ["celo", "tempo"])]
    #[cfg_attr(feature = "optimism", arg(conflicts_with = "optimism"))]
    #[cfg_attr(feature = "monad", arg(conflicts_with = "monad"))]
    #[cfg_attr(feature = "base", arg(conflicts_with = "base"))]
    #[serde(default)]
    pub(crate) network: Option<NetworkVariant>,
    /// Enable Celo network features.
    #[arg(help_heading = "Networks", long, conflicts_with_all = ["network", "tempo"])]
    #[cfg_attr(feature = "optimism", arg(conflicts_with = "optimism"))]
    #[cfg_attr(feature = "monad", arg(conflicts_with = "monad"))]
    #[cfg_attr(feature = "base", arg(conflicts_with = "base"))]
    celo: bool,
    /// Enable Optimism network features (deprecated: use --network optimism).
    #[cfg(feature = "optimism")]
    #[arg(long, hide = true, conflicts_with_all = ["network", "celo", "tempo"])]
    #[cfg_attr(feature = "monad", arg(conflicts_with = "monad"))]
    #[cfg_attr(feature = "base", arg(conflicts_with = "base"))]
    // Deserialize-only legacy alias: accepted in foundry.toml but never serialized — the
    // canonical form is `network = "optimism"`.
    #[serde(default)]
    pub(crate) optimism: bool,
    /// Enable Tempo network features (deprecated: use --network tempo).
    #[arg(long, hide = true, conflicts_with_all = ["network", "celo"])]
    #[cfg_attr(feature = "optimism", arg(conflicts_with = "optimism"))]
    #[cfg_attr(feature = "monad", arg(conflicts_with = "monad"))]
    #[cfg_attr(feature = "base", arg(conflicts_with = "base"))]
    // Deserialize-only legacy alias: accepted in foundry.toml but never serialized — the
    // canonical form is `network = "tempo"`.
    #[serde(default)]
    tempo: bool,
    /// Enable Monad network features (deprecated: use --network monad).
    #[cfg(feature = "monad")]
    #[arg(long, hide = true, conflicts_with_all = ["network", "celo", "tempo"])]
    #[cfg_attr(feature = "optimism", arg(conflicts_with = "optimism"))]
    #[cfg_attr(feature = "base", arg(conflicts_with = "base"))]
    // Deserialize-only legacy alias: accepted in foundry.toml but never serialized - the
    // canonical form is `network = "monad"`.
    #[serde(default)]
    monad: bool,
    /// Enable Base network features (deprecated: use --network base).
    #[cfg(feature = "base")]
    #[arg(long, hide = true, conflicts_with_all = ["network", "celo", "tempo"])]
    #[cfg_attr(feature = "optimism", arg(conflicts_with = "optimism"))]
    #[cfg_attr(feature = "monad", arg(conflicts_with = "monad"))]
    // Deserialize-only legacy alias: accepted in foundry.toml but never serialized — the
    // canonical form is `network = "base"`.
    #[serde(default)]
    base: bool,
    /// Whether to bypass prevrandao.
    #[arg(skip)]
    #[serde(default)]
    bypass_prevrandao: bool,
}

// Custom `Serialize` impl: always emits the *resolved* network as the canonical
// `network = "..."` field, and never emits legacy network aliases. This avoids contradictory
// canonical and legacy selectors, and ensures old foundry.toml keys round-trip canonically.
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
    /// Validates that all configured network selectors resolve to the same execution profile.
    ///
    /// Canonical and legacy selectors for the same family remain compatible. Selectors for
    /// different families are rejected instead of being silently resolved by field priority.
    pub fn validate(&self) -> Result<(), String> {
        let mut selectors = Vec::new();
        if let Some(network) = self.network {
            selectors.push((network.name(), format!("network = \"{}\"", network.name())));
        }
        if self.celo {
            selectors.push(("celo", "celo = true".to_string()));
        }
        #[cfg(feature = "optimism")]
        if self.optimism {
            selectors.push(("optimism", "optimism = true".to_string()));
        }
        if self.tempo {
            selectors.push(("tempo", "tempo = true".to_string()));
        }
        #[cfg(feature = "monad")]
        if self.monad {
            selectors.push(("monad", "monad = true".to_string()));
        }
        #[cfg(feature = "base")]
        if self.base {
            selectors.push(("base", "base = true".to_string()));
        }

        if let Some((family, selector)) = selectors.first()
            && let Some((_, conflicting)) =
                selectors.iter().find(|(candidate, _)| candidate != family)
        {
            return Err(format!(
                "network selectors `{selector}` and `{conflicting}` conflict; select only one \
                 network"
            ));
        }

        Ok(())
    }

    pub fn with_ethereum() -> Self {
        Self { network: Some(NetworkVariant::Ethereum), ..Default::default() }
    }

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

    #[cfg(feature = "base")]
    pub fn with_base() -> Self {
        Self { network: Some(NetworkVariant::Base), base: true, ..Default::default() }
    }

    pub const fn is_tempo(&self) -> bool {
        if let Some(network) = self.resolved_network() { network.is_tempo() } else { false }
    }

    /// Returns whether Optimism network features are enabled.
    ///
    /// Always returns `false` when built without the `optimism` feature.
    pub const fn is_optimism(&self) -> bool {
        if let Some(network) = self.resolved_network() { network.is_optimism() } else { false }
    }

    #[cfg(feature = "monad")]
    pub const fn is_monad(&self) -> bool {
        if let Some(network) = self.resolved_network() { network.is_monad() } else { false }
    }

    #[cfg(not(feature = "monad"))]
    pub const fn is_monad(&self) -> bool {
        false
    }

    #[cfg(feature = "base")]
    pub const fn is_base(&self) -> bool {
        matches!(self.resolved_network(), Some(NetworkVariant::Base))
    }

    /// Coerces `hardfork` into this network's family.
    ///
    /// Execution and fee rules apply the same lossy `From` conversions, so a cross-namespace
    /// override such as `--hardfork prague` on a Tempo node runs as a Tempo hardfork. Callers that
    /// need to describe what actually executed, like trace decoding, go through here rather than
    /// carrying the configured value.
    pub fn executed_hardfork(&self, hardfork: FoundryHardfork) -> FoundryHardfork {
        if self.is_tempo() {
            return TempoHardfork::from(hardfork).into();
        }
        #[cfg(feature = "monad")]
        if self.is_monad() {
            return MonadHardfork::from(hardfork).into();
        }
        #[cfg(feature = "base")]
        if self.is_base() {
            return BaseUpgrade::from(hardfork).into();
        }
        hardfork
    }

    pub const fn is_celo(&self) -> bool {
        self.celo
    }

    /// Returns the resolved network variant, folding legacy flags.
    pub const fn resolved_network(&self) -> Option<NetworkVariant> {
        if let Some(n) = self.network {
            return Some(n);
        }
        #[cfg(feature = "base")]
        if self.base {
            return Some(NetworkVariant::Base);
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

    /// Returns whether a network family was selected in this configuration.
    pub const fn has_network_selection(&self) -> bool {
        self.celo || self.resolved_network().is_some()
    }

    /// Returns the execution family represented by this configuration.
    pub const fn execution_family_name(&self) -> &'static str {
        self.execution_network().name()
    }

    /// Returns a label for the complete execution configuration.
    pub const fn execution_profile_name(&self) -> &'static str {
        if self.celo { "celo" } else { self.execution_family_name() }
    }

    /// Returns the concrete execution network, treating an unresolved configuration as Ethereum.
    pub const fn execution_network(&self) -> NetworkVariant {
        if let Some(network) = self.resolved_network() { network } else { NetworkVariant::Ethereum }
    }

    /// Returns whether both configurations can use the same instantiated EVM backend.
    pub fn has_same_execution_profile(&self, other: &Self) -> bool {
        self.celo == other.celo
            && match (self.resolved_network(), other.resolved_network()) {
                (Some(left), Some(right)) => left == right,
                (Some(left), None) => left.is_ethereum(),
                (None, Some(right)) => right.is_ethereum(),
                (None, None) => true,
            }
    }

    /// Returns whether this execution configuration can use `source` as a fork state source
    /// without rebuilding the instantiated EVM.
    ///
    /// Monad uses a distinct EVM factory and instruction provider, so forks cannot cross the
    /// Monad boundary. Existing non-Monad fork-source compatibility remains unchanged.
    pub const fn supports_fork_source(&self, source: &Self) -> bool {
        // Base also has its own EVM factory, so its boundary is impassable too.
        #[cfg(feature = "base")]
        if self.is_base() != source.is_base() {
            return false;
        }
        self.is_monad() == source.is_monad()
    }

    /// Returns the name of the currently active non-Ethereum network, or `None` for plain Ethereum.
    pub fn active_network_name(&self) -> Option<&'static str> {
        self.resolved_network().and_then(|network| network.hardfork_namespace())
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

    /// Calculates the blob excess gas inherited by the next block.
    ///
    /// OP Stack headers use the blob fields for protocol metadata rather than EIP-4844 blobs, so
    /// their excess blob gas remains zero. Other execution profiles use the configured Ethereum
    /// blob schedule.
    pub fn next_block_blob_excess_gas(
        &self,
        blob_params: BlobParams,
        parent_excess_blob_gas: u64,
        parent_blob_gas_used: u64,
        parent_base_fee: u64,
    ) -> u64 {
        if self.is_optimism() {
            return 0;
        }
        blob_params.next_block_excess_blob_gas_osaka(
            parent_excess_blob_gas,
            parent_blob_gas_used,
            parent_base_fee,
        )
    }

    /// Returns contract size limits for networks that override Ethereum defaults.
    #[cfg(feature = "monad")]
    pub fn contract_size_limits(&self) -> Option<NetworkContractSizeLimits> {
        self.is_monad().then_some(NetworkContractSizeLimits {
            runtime: monad_revm::MONAD_MAX_CODE_SIZE,
            initcode: monad_revm::MONAD_MAX_INITCODE_SIZE,
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

    /// Infers an execution profile from `chain_id` unless one was selected explicitly.
    ///
    /// Unknown chain IDs preserve the unresolved configuration so endpoint metadata can still
    /// identify their execution profile. A known chain whose feature is unavailable is rejected
    /// instead of silently falling back to Ethereum semantics.
    pub fn try_with_chain_id(self, chain_id: u64) -> Result<Self, String> {
        if self.has_network_selection() {
            return Ok(self);
        }

        match NetworkVariant::from_known_chain_id(chain_id).map_err(|error| {
            format!("cannot infer execution network from chain ID {chain_id}: {error}")
        })? {
            Some(network) => Ok(self.with_rpc_identity(network, chain_id)),
            None => Ok(self),
        }
    }

    /// Best-effort execution-profile inference for trusted, programmatic chain IDs.
    ///
    /// User-provided chain IDs must be handled with [`Self::try_with_chain_id`] so an unavailable
    /// feature cannot silently select Ethereum semantics.
    pub fn with_chain_id(self, chain_id: u64) -> Self {
        self.try_with_chain_id(chain_id).unwrap_or(self)
    }

    /// Applies an RPC endpoint's resolved EVM family to this configuration.
    ///
    /// Successful endpoint metadata is authoritative, including when a local Anvil uses a
    /// well-known chain ID as an execution override. Orthogonal settings are preserved.
    pub fn with_rpc_network(self, network: NetworkVariant) -> Self {
        let mut resolved = if network.is_ethereum() { Self::default() } else { network.into() };
        resolved.bypass_prevrandao = self.bypass_prevrandao;
        resolved
    }

    /// Applies an RPC endpoint's execution family and chain-specific Ethereum configuration.
    ///
    /// The reported family is authoritative. A known Celo chain ID additionally enables Celo's
    /// precompiles because Celo shares the Ethereum EVM factory rather than having its own
    /// [`NetworkVariant`].
    pub fn with_rpc_identity(self, network: NetworkVariant, chain_id: ChainId) -> Self {
        let mut resolved = self.with_rpc_network(network);
        if network.is_ethereum()
            && matches!(
                Chain::from_id(chain_id).named(),
                Some(NamedChain::Celo | NamedChain::CeloSepolia)
            )
        {
            resolved.celo = true;
        }
        resolved
    }

    /// Returns the canonical endpoint-visible execution profile.
    pub fn canonical_execution_profile(self) -> Self {
        if self.is_celo() {
            Self::with_celo()
        } else if let Some(network) = self.resolved_network()
            && !network.is_ethereum()
        {
            network.into()
        } else {
            Self::default()
        }
    }

    /// Applies an authoritative execution profile while preserving orthogonal settings.
    pub fn with_execution_profile(self, profile: Self) -> Self {
        let mut resolved = if profile.is_celo() {
            Self::with_celo()
        } else {
            profile.resolved_network().map(Into::into).unwrap_or_default()
        };
        resolved.bypass_prevrandao = self.bypass_prevrandao;
        resolved
    }

    /// Applies an endpoint-reported execution profile while preserving orthogonal settings.
    pub fn with_rpc_profile(self, profile: Self) -> Self {
        self.with_execution_profile(profile.canonical_execution_profile())
    }

    /// Parses the execution profile reported by `anvil_nodeInfo`.
    pub fn from_node_info_profile(profile: &str) -> Result<Self, String> {
        if profile == "celo" {
            return Ok(Self::with_celo());
        }
        let network = NetworkVariant::from_node_info_name(profile)?;
        Ok(if network.is_ethereum() { Self::default() } else { network.into() })
    }

    /// Resolves an RPC endpoint's complete execution profile.
    ///
    /// Explicit metadata is authoritative. When metadata is absent or omits the profile, a
    /// caller-supplied explicit profile takes precedence over chain-ID inference. Otherwise,
    /// legacy Anvil responses and ordinary RPC endpoints recover Celo from a canonical Celo chain
    /// ID and preserve the historical behavior for custom chain IDs.
    pub fn from_rpc_identity_profile_with_fallback(
        chain_id: ChainId,
        node_info_profile: Option<Option<&str>>,
        unknown_fallback: Option<Self>,
    ) -> Result<Option<Self>, String> {
        let known_profile = || {
            NetworkVariant::from_known_chain_id(chain_id).map(|network| {
                network.map(|network| Self::default().with_rpc_identity(network, chain_id))
            })
        };
        let fallback_is_explicit =
            unknown_fallback.is_some_and(|fallback| fallback.has_network_selection());
        let fallback = unknown_fallback.map(Self::canonical_execution_profile);
        if let Some(Some(profile)) = node_info_profile {
            return Self::from_node_info_profile(profile).map(Some);
        }
        if fallback_is_explicit && let Some(fallback) = fallback {
            return Ok(Some(fallback));
        }
        match node_info_profile {
            Some(None) => Ok(known_profile()?.or(fallback).or(Some(Self::default()))),
            None => Ok(known_profile()?.or(fallback)),
            Some(Some(_)) => unreachable!(),
        }
    }

    /// Validates `hardfork` against the current `NetworkConfigs` and, if consistent, returns an
    /// updated instance with the network implied by the enabled hardfork.
    ///
    /// Returns `Err` when the hardfork's network family conflicts with the configured one.
    pub fn normalize_for_hardfork(self, hardfork: FoundryHardfork) -> Result<Self, String> {
        if self.has_network_selection() {
            let configured = self.execution_network();
            if configured.hardfork_namespace() != hardfork.namespace() {
                return Err(format!(
                    "hardfork `{}` conflicts with network config `{}`",
                    String::from(hardfork),
                    self.execution_profile_name(),
                ));
            }
        }

        let network = match hardfork {
            FoundryHardfork::Ethereum(_) => self,
            FoundryHardfork::Tempo(_) => Self::with_tempo(),
            #[cfg(feature = "base")]
            FoundryHardfork::Base(_) => Self::with_base(),
            #[cfg(feature = "optimism")]
            FoundryHardfork::Optimism(_) => Self::with_optimism(),
            #[cfg(feature = "monad")]
            FoundryHardfork::Monad(_) => Self::with_monad(),
        };

        Ok(network)
    }

    /// Inject precompiles for configured networks.
    pub fn inject_precompiles(self, precompiles: &mut PrecompilesMap) {
        if self.is_celo() {
            precompiles.apply_precompile(&CELO_TRANSFER_ADDRESS, move |_| {
                Some(celo::transfer::precompile())
            });
        }
    }

    /// Returns precompile labels for configured networks at the given hardfork, to be used in
    /// traces.
    pub fn precompiles_label(self, hardfork: Option<FoundryHardfork>) -> AddressHashMap<String> {
        let mut labels = AddressHashMap::default();
        if self.is_celo() {
            labels.insert(CELO_TRANSFER_ADDRESS, CELO_TRANSFER_LABEL.to_string());
        }
        if self.is_tempo() {
            let tempo_hardfork = hardfork.and_then(TempoHardfork::from_foundry_hardfork);
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
            let monad_hardfork = hardfork.and_then(MonadHardfork::from_foundry_hardfork);
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
        #[cfg(feature = "base")]
        if self.is_base() {
            let base_upgrade =
                hardfork.and_then(BaseSpecId::from_foundry_hardfork).map(|spec| spec.upgrade());
            labels.extend(
                BASE_PRECOMPILES
                    .iter()
                    .copied()
                    .filter(|(_, address)| {
                        base_upgrade
                            .is_none_or(|upgrade| is_base_precompile_active_at(*address, upgrade))
                    })
                    .map(|(label, address)| (address, label.to_string())),
            );
        }
        labels
    }

    /// Returns precompiles for configured networks at the given hardfork.
    pub fn precompiles(self, hardfork: Option<FoundryHardfork>) -> BTreeMap<String, Address> {
        let mut precompiles = BTreeMap::new();
        if self.is_celo() {
            precompiles
                .insert(PRECOMPILE_ID_CELO_TRANSFER.name().to_string(), CELO_TRANSFER_ADDRESS);
        }
        if self.is_tempo() {
            let tempo_hardfork = hardfork.and_then(TempoHardfork::from_foundry_hardfork);
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
            let monad_hardfork = hardfork.and_then(MonadHardfork::from_foundry_hardfork);
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
        #[cfg(feature = "base")]
        if self.is_base() {
            let base_upgrade =
                hardfork.and_then(BaseSpecId::from_foundry_hardfork).map(|spec| spec.upgrade());
            precompiles.extend(
                BASE_PRECOMPILES
                    .iter()
                    .copied()
                    .filter(|(_, address)| {
                        base_upgrade
                            .is_none_or(|upgrade| is_base_precompile_active_at(*address, upgrade))
                    })
                    .map(|(label, address)| (label.to_string(), address)),
            );
        }
        precompiles
    }
}

/// Applies the BSC P256 precompile active at the given timestamp.
pub fn apply_bsc_p256_precompile(
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

impl From<NetworkVariant> for NetworkConfigs {
    fn from(network: NetworkVariant) -> Self {
        match network {
            NetworkVariant::Ethereum => Self::with_ethereum(),
            NetworkVariant::Tempo => {
                Self { network: Some(network), tempo: true, ..Default::default() }
            }
            #[cfg(feature = "monad")]
            NetworkVariant::Monad => {
                Self { network: Some(network), monad: true, ..Default::default() }
            }
            #[cfg(feature = "base")]
            NetworkVariant::Base => {
                Self { network: Some(network), base: true, ..Default::default() }
            }
            #[cfg(feature = "optimism")]
            NetworkVariant::Optimism => {
                Self { network: Some(network), optimism: true, ..Default::default() }
            }
        }
    }
}

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
    address == monad_revm::staking::STAKING_ADDRESS
        || (address == monad_revm::reserve_balance::abi::RESERVE_BALANCE_ADDRESS
            && MonadHardfork::MonadNine.is_enabled_in(hardfork))
}

/// Returns whether a fixed Base precompile is active at `upgrade`.
#[cfg(feature = "base")]
pub fn is_base_precompile_active_at(address: Address, upgrade: BaseUpgrade) -> bool {
    if matches!(address, TxContextStorage::ADDRESS | NonceManagerStorage::ADDRESS) {
        upgrade >= BaseUpgrade::Cobalt
    } else if matches!(
        address,
        B20FactoryStorage::ADDRESS
            | ActivationRegistryStorage::ADDRESS
            | PolicyRegistryStorage::ADDRESS
    ) {
        upgrade >= BaseUpgrade::Beryl
    } else {
        false
    }
}

/// Returns the fixed Base precompiles active at `upgrade`.
#[cfg(feature = "base")]
pub fn active_base_precompiles(
    upgrade: BaseUpgrade,
) -> impl Iterator<Item = (&'static str, Address)> {
    BASE_PRECOMPILES
        .iter()
        .copied()
        .filter(move |(_, address)| is_base_precompile_active_at(*address, upgrade))
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
        assert!(!NetworkVariant::Ethereum.is_optimism());
        assert!(!NetworkVariant::Ethereum.is_tempo());
        assert!(NetworkVariant::Tempo.is_tempo());
        assert!(!NetworkVariant::Tempo.is_ethereum());
        assert!(!NetworkVariant::Tempo.is_optimism());

        #[cfg(feature = "monad")]
        {
            assert!(!NetworkVariant::Ethereum.is_monad());
            assert!(!NetworkVariant::Tempo.is_monad());
            assert!(NetworkVariant::Monad.is_monad());
            assert!(!NetworkVariant::Monad.is_ethereum());
            assert!(!NetworkVariant::Monad.is_optimism());
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

        #[cfg(feature = "base")]
        {
            assert!(NetworkVariant::Base.is_base());
            assert!(!NetworkVariant::Base.is_ethereum());
            assert!(!NetworkVariant::Base.is_optimism());
            assert!(!NetworkVariant::Base.is_tempo());
        }
    }

    #[test]
    #[cfg(feature = "monad")]
    fn fork_sources_only_isolate_monad() {
        let mut non_monad = vec![
            NetworkConfigs::default(),
            NetworkConfigs::with_ethereum(),
            NetworkConfigs::with_celo(),
            NetworkConfigs::with_tempo(),
        ];
        #[cfg(feature = "optimism")]
        non_monad.push(NetworkConfigs::with_optimism());

        for execution in &non_monad {
            for source in &non_monad {
                assert!(execution.supports_fork_source(source));
            }
            assert!(!execution.supports_fork_source(&NetworkConfigs::with_monad()));
            assert!(!NetworkConfigs::with_monad().supports_fork_source(execution));
        }
        assert!(NetworkConfigs::with_monad().supports_fork_source(&NetworkConfigs::with_monad()));
    }

    #[test]
    #[cfg(feature = "base")]
    fn fork_sources_isolate_base() {
        #[cfg_attr(not(any(feature = "optimism", feature = "monad")), allow(unused_mut))]
        let mut non_base = vec![
            NetworkConfigs::default(),
            NetworkConfigs::with_ethereum(),
            NetworkConfigs::with_celo(),
            NetworkConfigs::with_tempo(),
        ];
        #[cfg(feature = "optimism")]
        non_base.push(NetworkConfigs::with_optimism());
        #[cfg(feature = "monad")]
        non_base.push(NetworkConfigs::with_monad());

        for source in &non_base {
            assert!(!NetworkConfigs::with_base().supports_fork_source(source));
            assert!(!source.supports_fork_source(&NetworkConfigs::with_base()));
        }
        assert!(NetworkConfigs::with_base().supports_fork_source(&NetworkConfigs::with_base()));
    }

    #[test]
    fn known_chain_identity_does_not_guess_unknown_networks() {
        assert_eq!(NetworkVariant::from_known_chain_id(98_765_432).unwrap(), None);
        assert_eq!(
            NetworkVariant::from_known_chain_id(NamedChain::Mainnet as u64).unwrap(),
            Some(NetworkVariant::Ethereum)
        );
        assert_eq!(NetworkVariant::from(98_765_432), NetworkVariant::Ethereum);
    }

    #[test]
    fn fallible_chain_id_inference_preserves_unknown_chains() {
        let networks = NetworkConfigs { bypass_prevrandao: true, ..Default::default() };
        assert_eq!(networks.try_with_chain_id(98_765_432).unwrap(), networks);
    }

    #[test]
    #[cfg(not(feature = "monad"))]
    fn fallible_chain_id_inference_rejects_disabled_monad() {
        for chain_id in [NamedChain::Monad as u64, NamedChain::MonadTestnet as u64] {
            let unavailable = "network family `monad` is not enabled in this build";
            let expected =
                format!("cannot infer execution network from chain ID {chain_id}: {unavailable}");
            assert_eq!(
                NetworkConfigs::default().try_with_chain_id(chain_id).unwrap_err(),
                expected
            );
        }
    }

    #[test]
    #[cfg(not(feature = "optimism"))]
    fn fallible_chain_id_inference_rejects_disabled_optimism() {
        let chain_id = NamedChain::Optimism as u64;
        assert_eq!(
            NetworkConfigs::default().try_with_chain_id(chain_id).unwrap_err(),
            format!(
                "cannot infer execution network from chain ID {chain_id}: network family \
                 `optimism` is not enabled in this build"
            )
        );
    }

    #[test]
    fn explicit_ethereum_overrides_known_chain_inference() {
        let ethereum = NetworkConfigs::with_ethereum();
        for chain_id in [NamedChain::Monad as u64, NamedChain::Optimism as u64] {
            assert_eq!(ethereum.try_with_chain_id(chain_id).unwrap(), ethereum);
        }
    }

    #[test]
    fn fallible_chain_id_inference_preserves_orthogonal_configuration() {
        let networks = NetworkConfigs { bypass_prevrandao: true, ..Default::default() };
        let inferred = networks.try_with_chain_id(NamedChain::Tempo as u64).unwrap();

        assert!(inferred.is_tempo());
        assert!(inferred.bypass_prevrandao(NamedChain::Mainnet as u64));
    }

    #[test]
    #[cfg(feature = "optimism")]
    fn fallible_chain_id_inference_detects_optimism() {
        let chain_id = NamedChain::Optimism as u64;
        assert_eq!(NetworkVariant::from(chain_id), NetworkVariant::Optimism);
        assert!(NetworkConfigs::default().try_with_chain_id(chain_id).unwrap().is_optimism());
    }

    #[test]
    fn rpc_identity_preserves_orthogonal_configuration() {
        let base = NetworkConfigs { bypass_prevrandao: true, ..NetworkConfigs::default() };
        let ethereum = base.with_rpc_network(NetworkVariant::Ethereum);
        assert!(ethereum.bypass_prevrandao(NamedChain::Mainnet as u64));

        let ethereum = NetworkConfigs::default().with_rpc_network(NetworkVariant::Ethereum);
        assert_eq!(ethereum, NetworkConfigs::default());

        assert!(
            NetworkConfigs::default()
                .with_rpc_identity(NetworkVariant::Ethereum, NamedChain::Celo as u64)
                .is_celo()
        );
        assert!(
            !NetworkConfigs::default()
                .with_rpc_identity(NetworkVariant::Ethereum, NamedChain::Monad as u64)
                .is_monad()
        );
    }

    #[test]
    fn rpc_metadata_overrides_known_execution_chain_identity() {
        assert_eq!(
            NetworkVariant::from_rpc_identity(NamedChain::Mainnet as u64, Some(Some("tempo")))
                .unwrap(),
            Some(NetworkVariant::Tempo)
        );
        assert_eq!(
            NetworkVariant::from_rpc_identity(NamedChain::Tempo as u64, Some(None)).unwrap(),
            Some(NetworkVariant::Tempo)
        );
        assert_eq!(NetworkVariant::from_rpc_identity(98_765_432, None).unwrap(), None);
        assert_eq!(
            NetworkVariant::from_rpc_identity(98_765_432, Some(None)).unwrap(),
            Some(NetworkVariant::Ethereum)
        );
        #[cfg(feature = "optimism")]
        assert_eq!(
            NetworkVariant::from_rpc_identity(NamedChain::Optimism as u64, Some(None)).unwrap(),
            Some(NetworkVariant::Optimism)
        );
        assert_eq!(
            NetworkVariant::from_rpc_identity(NamedChain::Celo as u64, Some(None)).unwrap(),
            Some(NetworkVariant::Ethereum)
        );
    }

    #[test]
    fn rpc_profile_distinguishes_celo_from_ethereum_factory() {
        let custom_chain_id = 98_765_432;
        assert!(
            NetworkConfigs::from_rpc_identity_profile_with_fallback(
                custom_chain_id,
                Some(Some("celo")),
                None,
            )
            .unwrap()
            .unwrap()
            .is_celo()
        );
        assert!(
            !NetworkConfigs::from_rpc_identity_profile_with_fallback(
                NamedChain::Celo as u64,
                Some(Some("ethereum")),
                None,
            )
            .unwrap()
            .unwrap()
            .is_celo()
        );
        assert!(
            NetworkConfigs::from_rpc_identity_profile_with_fallback(
                NamedChain::Celo as u64,
                Some(None),
                None,
            )
            .unwrap()
            .unwrap()
            .is_celo()
        );
        assert!(
            NetworkConfigs::from_rpc_identity_profile_with_fallback(
                custom_chain_id,
                None,
                Some(NetworkConfigs::with_celo()),
            )
            .unwrap()
            .unwrap()
            .is_celo()
        );
    }

    #[test]
    fn rpc_profile_keeps_moonbeam_on_default_ethereum() {
        let profile = NetworkConfigs::from_rpc_identity_profile_with_fallback(
            NamedChain::Moonbeam as u64,
            None,
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(profile, NetworkConfigs::default());
        assert!(profile.bypass_prevrandao(NamedChain::Moonbeam as u64));
    }

    #[test]
    fn default_fallback_still_infers_known_execution_profile() {
        for node_info in [None, Some(None)] {
            assert_eq!(
                NetworkConfigs::from_rpc_identity_profile_with_fallback(
                    NamedChain::Tempo as u64,
                    node_info,
                    Some(NetworkConfigs::default()),
                )
                .unwrap(),
                Some(NetworkConfigs::with_tempo())
            );
        }
    }

    #[test]
    fn explicit_ethereum_overrides_known_execution_profile() {
        for node_info in [None, Some(None)] {
            assert_eq!(
                NetworkConfigs::from_rpc_identity_profile_with_fallback(
                    NamedChain::Tempo as u64,
                    node_info,
                    Some(NetworkConfigs::with_ethereum()),
                )
                .unwrap(),
                Some(NetworkConfigs::default())
            );
        }
    }

    #[test]
    #[cfg(not(feature = "monad"))]
    fn explicit_ethereum_overrides_disabled_monad_without_node_info() {
        assert_eq!(
            NetworkConfigs::from_rpc_identity_profile_with_fallback(
                NamedChain::Monad as u64,
                None,
                Some(NetworkConfigs::with_ethereum()),
            )
            .unwrap(),
            Some(NetworkConfigs::default())
        );
    }

    #[test]
    #[cfg(not(feature = "monad"))]
    fn explicit_ethereum_overrides_disabled_monad_with_legacy_node_info() {
        assert_eq!(
            NetworkConfigs::from_rpc_identity_profile_with_fallback(
                NamedChain::Monad as u64,
                Some(None),
                Some(NetworkConfigs::with_ethereum()),
            )
            .unwrap(),
            Some(NetworkConfigs::default())
        );
    }

    #[test]
    #[cfg(not(feature = "monad"))]
    fn disabled_monad_without_explicit_profile_still_errors() {
        for node_info in [None, Some(None)] {
            assert_eq!(
                NetworkConfigs::from_rpc_identity_profile_with_fallback(
                    NamedChain::Monad as u64,
                    node_info,
                    None,
                )
                .unwrap_err(),
                "network family `monad` is not enabled in this build"
            );
        }
    }

    #[test]
    #[cfg(feature = "monad")]
    fn rpc_identity_fallback_preserves_explicit_custom_networks() {
        let custom_chain_id = 98_765_432;
        for node_info in [None, Some(None)] {
            assert_eq!(
                NetworkVariant::from_rpc_identity_with_fallback(
                    custom_chain_id,
                    node_info,
                    Some(NetworkVariant::Monad),
                )
                .unwrap(),
                Some(NetworkVariant::Monad)
            );
        }

        assert_eq!(
            NetworkVariant::from_rpc_identity_with_fallback(
                NamedChain::Mainnet as u64,
                Some(None),
                Some(NetworkVariant::Monad),
            )
            .unwrap(),
            Some(NetworkVariant::Ethereum)
        );
        assert_eq!(
            NetworkVariant::from_rpc_identity_with_fallback(
                custom_chain_id,
                Some(Some("tempo")),
                Some(NetworkVariant::Monad),
            )
            .unwrap(),
            Some(NetworkVariant::Tempo)
        );
    }

    #[test]
    fn network_selection_distinguishes_default_and_explicit_ethereum() {
        assert!(!NetworkConfigs::default().has_network_selection());
        assert!(NetworkConfigs::with_ethereum().has_network_selection());
        assert!(NetworkConfigs::with_celo().has_network_selection());
    }

    #[test]
    fn celo_uses_ethereum_factory_with_distinct_precompiles() {
        assert!(
            !NetworkConfigs::with_celo().has_same_execution_profile(&NetworkConfigs::default())
        );
        assert!(
            NetworkConfigs::with_ethereum().has_same_execution_profile(&NetworkConfigs::default())
        );
        assert_eq!(NetworkConfigs::with_celo().execution_family_name(), "ethereum");
        assert_eq!(NetworkConfigs::with_celo().execution_profile_name(), "celo");
    }

    #[test]
    fn authoritative_execution_profile_preserves_orthogonal_settings() {
        let inline = NetworkConfigs { bypass_prevrandao: true, ..NetworkConfigs::with_tempo() };

        #[cfg_attr(not(any(feature = "optimism", feature = "monad")), allow(unused_mut))]
        let mut profiles = vec![
            NetworkConfigs::with_ethereum(),
            NetworkConfigs::with_tempo(),
            NetworkConfigs::with_celo(),
        ];
        #[cfg(feature = "optimism")]
        profiles.push(NetworkVariant::Optimism.into());
        #[cfg(feature = "monad")]
        profiles.push(NetworkConfigs::with_monad());

        for profile in profiles {
            let resolved = inline.with_execution_profile(profile);
            assert!(resolved.has_same_execution_profile(&profile));
            assert!(resolved.has_network_selection());
            assert!(resolved.bypass_prevrandao(NamedChain::Mainnet as u64));
        }

        let ethereum = inline.with_execution_profile(NetworkConfigs::with_ethereum());
        assert_eq!(
            ethereum.try_with_chain_id(NamedChain::Tempo as u64).unwrap(),
            ethereum,
            "an authoritative Ethereum profile must prevent later endpoint inference",
        );

        let rpc_ethereum = inline.with_rpc_profile(NetworkConfigs::with_ethereum());
        assert!(!rpc_ethereum.has_network_selection());
    }

    #[test]
    fn chain_id_inference_preserves_explicit_networks() {
        assert!(
            NetworkConfigs::default().try_with_chain_id(NamedChain::Celo as u64).unwrap().is_celo()
        );
        let celo = NetworkConfigs { bypass_prevrandao: true, ..Default::default() }
            .try_with_chain_id(NamedChain::Celo as u64)
            .unwrap();
        assert!(celo.is_celo());
        assert!(celo.bypass_prevrandao(NamedChain::Mainnet as u64));

        let explicit = [
            NetworkConfigs::with_ethereum(),
            NetworkConfigs::with_tempo(),
            NetworkConfigs::with_celo(),
        ];
        for networks in explicit {
            assert_eq!(networks.try_with_chain_id(NamedChain::Celo as u64).unwrap(), networks);
        }

        #[cfg(feature = "monad")]
        {
            let monad = NetworkConfigs::with_monad();
            assert_eq!(monad.try_with_chain_id(NamedChain::Celo as u64).unwrap(), monad);
        }
    }

    #[test]
    #[cfg(feature = "monad")]
    fn rpc_metadata_identifies_custom_monad_networks() {
        assert_eq!(NetworkVariant::from_node_info_name("monad").unwrap(), NetworkVariant::Monad);
        assert!(NetworkConfigs::default().with_rpc_network(NetworkVariant::Monad).is_monad());
    }

    #[test]
    #[cfg(feature = "monad")]
    fn parses_endpoint_hardfork_in_network_namespace() {
        assert_eq!(
            NetworkVariant::Monad.parse_hardfork("MonadEight").unwrap(),
            FoundryHardfork::Monad(MonadHardfork::MonadEight)
        );
    }

    #[test]
    fn rpc_metadata_rejects_unknown_network_family() {
        assert_eq!(
            NetworkVariant::from_node_info_name("unknown").unwrap_err(),
            "unsupported network family `unknown` reported by fork endpoint"
        );
    }

    #[test]
    fn explicit_ethereum_families_reject_namespaced_hardforks() {
        #[cfg_attr(not(feature = "monad"), allow(unused_mut))]
        let mut incompatible = vec![FoundryHardfork::Tempo(TempoHardfork::T0)];
        #[cfg(feature = "monad")]
        incompatible.push(FoundryHardfork::Monad(MonadHardfork::MonadEight));

        for networks in [NetworkConfigs::with_ethereum(), NetworkConfigs::with_celo()] {
            for hardfork in &incompatible {
                assert_eq!(
                    networks.normalize_for_hardfork(*hardfork).unwrap_err(),
                    format!(
                        "hardfork `{}` conflicts with network config `{}`",
                        String::from(*hardfork),
                        networks.execution_profile_name()
                    )
                );
            }
        }

        let celo = NetworkConfigs::with_celo();
        assert_eq!(
            celo.normalize_for_hardfork(FoundryHardfork::Ethereum(
                foundry_evm_hardforks::EthereumHardfork::Prague
            ))
            .unwrap(),
            celo
        );
    }

    #[cfg(feature = "base")]
    #[test]
    fn base_precompile_labels_follow_upgrade_boundaries() {
        let config = NetworkConfigs::with_base();

        assert!(config.precompiles_label(Some(BaseUpgrade::Azul.into())).is_empty());

        let beryl = config.precompiles_label(Some(BaseUpgrade::Beryl.into()));
        assert_eq!(beryl.get(&B20FactoryStorage::ADDRESS), Some(&"B20Factory".to_string()));
        assert_eq!(
            beryl.get(&ActivationRegistryStorage::ADDRESS),
            Some(&"ActivationRegistry".to_string())
        );
        assert_eq!(beryl.get(&PolicyRegistryStorage::ADDRESS), Some(&"PolicyRegistry".to_string()));
        assert!(!beryl.contains_key(&TxContextStorage::ADDRESS));
        assert!(!beryl.contains_key(&NonceManagerStorage::ADDRESS));

        let cobalt = config.precompiles_label(Some(BaseUpgrade::Cobalt.into()));
        assert_eq!(cobalt.len(), BASE_PRECOMPILES.len());
        assert_eq!(config.precompiles_label(None).len(), BASE_PRECOMPILES.len());

        // The name-keyed precompile map must honor the same upgrade boundaries.
        assert!(config.precompiles(Some(BaseUpgrade::Azul.into())).is_empty());
        let beryl = config.precompiles(Some(BaseUpgrade::Beryl.into()));
        assert_eq!(beryl.get("B20Factory"), Some(&B20FactoryStorage::ADDRESS));
        assert!(!beryl.contains_key("NonceManager"));
        assert_eq!(
            config.precompiles(Some(BaseUpgrade::Cobalt.into())).get("NonceManager"),
            Some(&NonceManagerStorage::ADDRESS)
        );

        // A non-Base profile must not pick up Base labels even when an upgrade is supplied.
        assert!(
            NetworkConfigs::default()
                .precompiles_label(Some(BaseUpgrade::Cobalt.into()))
                .is_empty()
        );
    }

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
        apply_bsc_p256_precompile(
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
            cfg.precompiles(None).get("TIP20ChannelReserve"),
            Some(&TIP20_CHANNEL_RESERVE_ADDRESS)
        );
        assert!(
            !cfg.precompiles(Some(TempoHardfork::T4.into())).contains_key("TIP20ChannelReserve")
        );
        assert!(
            !cfg.precompiles(Some(TempoHardfork::T4.into())).contains_key("ReceivePolicyGuard")
        );
        assert!(!cfg.precompiles(Some(TempoHardfork::T2.into())).contains_key("AddressRegistry"));
        assert!(!cfg.precompiles(Some(TempoHardfork::T2.into())).contains_key("SignatureVerifier"));
        assert_eq!(
            cfg.precompiles(Some(TempoHardfork::T3.into())).get("AddressRegistry"),
            Some(&ADDRESS_REGISTRY_ADDRESS)
        );
        assert_eq!(
            cfg.precompiles(Some(TempoHardfork::T3.into())).get("SignatureVerifier"),
            Some(&SIGNATURE_VERIFIER_ADDRESS)
        );
        assert_eq!(
            cfg.precompiles_label(Some(TempoHardfork::T5.into()))
                .get(&TIP20_CHANNEL_RESERVE_ADDRESS),
            Some(&"TIP20ChannelReserve".to_string())
        );
        assert!(cfg.precompiles_label(None).contains_key(&TIP20_CHANNEL_RESERVE_ADDRESS));
        assert!(
            !cfg.precompiles_label(Some(TempoHardfork::T5.into()))
                .contains_key(&RECEIVE_POLICY_GUARD_ADDRESS)
        );
        assert!(
            cfg.precompiles_label(Some(TempoHardfork::T6.into()))
                .contains_key(&RECEIVE_POLICY_GUARD_ADDRESS)
        );
    }

    #[test]
    #[cfg(feature = "monad")]
    fn canonical_monad_network_reports_hardfork_gated_precompiles() {
        let cfg = NetworkConfigs { network: Some(NetworkVariant::Monad), ..Default::default() };

        assert_eq!(
            cfg.precompiles(Some(MonadHardfork::MonadEight.into())).get("MonadStaking"),
            Some(&monad_revm::staking::STAKING_ADDRESS)
        );
        assert!(
            !cfg.precompiles(Some(MonadHardfork::MonadEight.into()))
                .contains_key("MonadReserveBalance")
        );
        assert_eq!(
            cfg.precompiles(Some(MonadHardfork::MonadNine.into())).get("MonadReserveBalance"),
            Some(&monad_revm::reserve_balance::abi::RESERVE_BALANCE_ADDRESS)
        );
        assert_eq!(
            cfg.precompiles_label(Some(MonadHardfork::MonadNine.into()))
                .get(&monad_revm::reserve_balance::abi::RESERVE_BALANCE_ADDRESS),
            Some(&"ReserveBalance".to_string())
        );
        assert!(
            cfg.precompiles_label(None)
                .contains_key(&monad_revm::reserve_balance::abi::RESERVE_BALANCE_ADDRESS)
        );
        assert!(
            !cfg.precompiles_label(Some(MonadHardfork::MonadEight.into()))
                .contains_key(&monad_revm::reserve_balance::abi::RESERVE_BALANCE_ADDRESS)
        );
    }

    #[test]
    fn storage_credits_precompile_activates_at_t7() {
        assert!(!is_tempo_precompile_active_at(STORAGE_CREDITS_ADDRESS, TempoHardfork::T6));
        assert!(is_tempo_precompile_active_at(STORAGE_CREDITS_ADDRESS, TempoHardfork::T7));
        assert!(TEMPO_PRECOMPILE_ADDRESSES.contains(&STORAGE_CREDITS_ADDRESS));

        // The hardfork-filtered precompile map must honor the same T7 activation.
        let cfg = NetworkConfigs { network: Some(NetworkVariant::Tempo), ..Default::default() };
        assert!(!cfg.precompiles(Some(TempoHardfork::T6.into())).contains_key("StorageCredits"));
        assert!(cfg.precompiles(Some(TempoHardfork::T7.into())).contains_key("StorageCredits"));
    }

    #[test]
    fn current_committee_precompile_activates_at_t8() {
        assert!(!is_tempo_precompile_active_at(CURRENT_COMMITTEE_ADDRESS, TempoHardfork::T7));
        assert!(is_tempo_precompile_active_at(CURRENT_COMMITTEE_ADDRESS, TempoHardfork::T8));
        assert!(TEMPO_PRECOMPILE_ADDRESSES.contains(&CURRENT_COMMITTEE_ADDRESS));

        let cfg = NetworkConfigs { network: Some(NetworkVariant::Tempo), ..Default::default() };
        assert!(!cfg.precompiles(Some(TempoHardfork::T7.into())).contains_key("CurrentCommittee"));
        assert!(cfg.precompiles(Some(TempoHardfork::T8.into())).contains_key("CurrentCommittee"));
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
        assert_eq!(limits.runtime, monad_revm::MONAD_MAX_CODE_SIZE);
        assert_eq!(limits.initcode, monad_revm::MONAD_MAX_INITCODE_SIZE);
        assert!(NetworkConfigs::default().contract_size_limits().is_none());
    }

    #[test]
    fn active_network_name_default_is_none() {
        let cfg = NetworkConfigs::default();
        assert_eq!(cfg.active_network_name(), None);
        assert!(!cfg.is_optimism());
    }

    /// Base chain IDs resolved to Optimism before Base support existed, and `is_optimism()` still
    /// covers them, so a build without the `base` feature must keep resolving them rather than
    /// erroring. Shipped release binaries are exactly that build.
    #[test]
    #[cfg(all(not(feature = "base"), feature = "optimism"))]
    fn chain_id_inference_falls_back_to_optimism_without_base() {
        for chain_id in [NamedChain::Base as u64, NamedChain::BaseSepolia as u64] {
            let configs = NetworkConfigs::default()
                .try_with_chain_id(chain_id)
                .unwrap_or_else(|error| panic!("chain ID {chain_id} must still resolve: {error}"));
            assert!(configs.is_optimism(), "chain ID {chain_id} must resolve to Optimism");
        }
    }

    #[test]
    #[cfg(not(feature = "base"))]
    fn node_info_rejects_disabled_base() {
        assert_eq!(
            NetworkVariant::from_node_info_name("base").unwrap_err(),
            "network family `base` is not enabled in this build"
        );
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
    fn validates_flattened_network_selectors() {
        let valid = [
            NetworkConfigs::default(),
            NetworkConfigs::with_ethereum(),
            NetworkConfigs::with_celo(),
            NetworkConfigs::with_tempo(),
            NetworkConfigs {
                network: Some(NetworkVariant::Tempo),
                tempo: true,
                ..Default::default()
            },
        ];
        for networks in valid {
            networks.validate().unwrap();
        }

        let conflicts = [
            (
                NetworkConfigs {
                    network: Some(NetworkVariant::Tempo),
                    celo: true,
                    ..Default::default()
                },
                "network selectors `network = \"tempo\"` and `celo = true` conflict",
            ),
            (
                NetworkConfigs { celo: true, tempo: true, ..Default::default() },
                "network selectors `celo = true` and `tempo = true` conflict",
            ),
        ];
        for (networks, expected) in conflicts {
            assert!(networks.validate().unwrap_err().contains(expected));
        }
    }

    #[test]
    #[cfg(feature = "monad")]
    fn validates_flattened_monad_network_selectors() {
        for networks in [
            NetworkConfigs::with_monad(),
            NetworkConfigs {
                network: Some(NetworkVariant::Monad),
                monad: true,
                ..Default::default()
            },
        ] {
            networks.validate().unwrap();
        }

        let conflicts = [
            (
                NetworkConfigs {
                    network: Some(NetworkVariant::Monad),
                    celo: true,
                    ..Default::default()
                },
                "network selectors `network = \"monad\"` and `celo = true` conflict",
            ),
            (
                NetworkConfigs {
                    network: Some(NetworkVariant::Monad),
                    tempo: true,
                    ..Default::default()
                },
                "network selectors `network = \"monad\"` and `tempo = true` conflict",
            ),
            (
                NetworkConfigs { celo: true, monad: true, ..Default::default() },
                "network selectors `celo = true` and `monad = true` conflict",
            ),
            (
                NetworkConfigs { tempo: true, monad: true, ..Default::default() },
                "network selectors `tempo = true` and `monad = true` conflict",
            ),
        ];
        for (networks, expected) in conflicts {
            assert!(networks.validate().unwrap_err().contains(expected));
        }
    }

    #[test]
    #[cfg(feature = "monad")]
    fn chain_id_detects_monad_network() {
        assert_eq!(NetworkVariant::from(143), NetworkVariant::Monad);
        assert_eq!(NetworkVariant::from(10143), NetworkVariant::Monad);

        assert!(NetworkConfigs::default().try_with_chain_id(143).unwrap().is_monad());
    }

    #[cfg(feature = "base")]
    mod base {
        use super::*;

        #[test]
        fn new_base_flag_equivalent_to_legacy() {
            let via_new =
                NetworkConfigs { network: Some(NetworkVariant::Base), ..Default::default() };
            let via_old = NetworkConfigs { base: true, ..Default::default() };
            assert_eq!(via_new.is_base(), via_old.is_base());
            assert_eq!(via_new.is_tempo(), via_old.is_tempo());
            assert_eq!(via_new.active_network_name(), via_old.active_network_name());
        }

        #[test]
        fn new_flag_wins_over_legacy_when_both_set() {
            // --network base --tempo: network field wins
            let cfg = NetworkConfigs {
                network: Some(NetworkVariant::Base),
                tempo: true,
                ..Default::default()
            };
            assert!(cfg.is_base());
            assert!(!cfg.is_tempo());
        }

        #[test]
        fn active_network_name_base() {
            let cfg = NetworkConfigs::with_base();
            assert_eq!(cfg.active_network_name(), Some("base"));
        }

        #[test]
        fn serde_roundtrip_base() {
            let original = NetworkConfigs::with_base();
            let json = serde_json::to_string(&original).unwrap();
            let restored: NetworkConfigs = serde_json::from_str(&json).unwrap();
            assert!(restored.is_base());
            assert!(!restored.is_tempo());
        }

        #[test]
        fn serde_base_field_deserialized() {
            let json_base = r#"{"network": "base", "celo": false, "bypass_prevrandao": false}"#;
            let cfg_base: NetworkConfigs = serde_json::from_str(json_base).unwrap();
            assert!(cfg_base.is_base());
        }

        #[test]
        fn chain_id_detects_base_networks() {
            assert_eq!(NetworkVariant::from(8453), NetworkVariant::Base);
            assert_eq!(NetworkVariant::from(84532), NetworkVariant::Base);
            assert!(NetworkConfigs::default().try_with_chain_id(8453).unwrap().is_base());
            assert_eq!(NetworkVariant::from_node_info_name("base").unwrap(), NetworkVariant::Base);
        }

        #[test]
        fn hardfork_infers_base_network() {
            assert_eq!(
                NetworkConfigs::default()
                    .normalize_for_hardfork(FoundryHardfork::Base(BaseUpgrade::Beryl))
                    .unwrap()
                    .resolved_network(),
                Some(NetworkVariant::Base)
            );
        }
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
        fn matching_optimism_selectors_are_valid() {
            NetworkConfigs::with_optimism().validate().unwrap();
        }

        #[test]
        fn active_network_name_optimism() {
            let cfg = NetworkConfigs::with_optimism();
            assert_eq!(cfg.active_network_name(), Some("optimism"));
        }

        #[test]
        fn conflicting_optimism_and_tempo_selectors_are_rejected() {
            let cfg = NetworkConfigs {
                network: Some(NetworkVariant::Optimism),
                tempo: true,
                ..Default::default()
            };
            assert_eq!(
                cfg.validate().unwrap_err(),
                "network selectors `network = \"optimism\"` and `tempo = true` conflict; select \
                 only one network"
            );
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

    #[test]
    fn executed_hardfork_follows_the_network_family() {
        // A cross-namespace override runs as the configured network's hardfork, so the value used
        // to describe execution has to be coerced the same way.
        let prague = FoundryHardfork::Ethereum(EthereumHardfork::Prague);
        assert_eq!(NetworkConfigs::default().executed_hardfork(prague), prague);
        assert_eq!(
            NetworkConfigs::with_tempo().executed_hardfork(prague),
            FoundryHardfork::Tempo(TempoHardfork::from(prague))
        );
    }
}
