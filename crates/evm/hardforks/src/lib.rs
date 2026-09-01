//! EVM hardfork definitions for Foundry.
//!
//! Provides [`FoundryHardfork`], a unified enum over Ethereum, Optimism, Tempo, and Monad hardforks
//! with `FromStr`/`Serialize`/`Deserialize` support for CLI and config usage.

use std::{
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use alloy_chains::Chain;
use alloy_rpc_types::BlockNumberOrTag;
use foundry_compilers::artifacts::EvmVersion;
#[cfg(feature = "optimism")]
use op_revm::OpSpecId;
use revm::primitives::hardfork::SpecId;
use serde::{Deserialize, Serialize};

pub use alloy_hardforks::EthereumHardfork;
#[cfg(feature = "optimism")]
pub use alloy_op_hardforks::OpHardfork;
#[cfg(feature = "base")]
pub use base_common_evm::BaseSpecId;
#[cfg(feature = "base")]
pub use base_common_genesis::BaseUpgrade;
#[cfg(feature = "monad")]
pub use monad_revm::MonadHardfork;
pub use tempo_hardfork::TempoHardfork;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(into = "String")]
pub enum FoundryHardfork {
    Ethereum(EthereumHardfork),
    #[cfg(feature = "optimism")]
    Optimism(OpHardfork),
    #[cfg(feature = "base")]
    Base(BaseUpgrade),
    Tempo(TempoHardfork),
    #[cfg(feature = "monad")]
    Monad(MonadHardfork),
}

impl From<FoundryHardfork> for String {
    fn from(fork: FoundryHardfork) -> Self {
        match fork {
            FoundryHardfork::Ethereum(h) => format!("{h}"),
            #[cfg(feature = "optimism")]
            FoundryHardfork::Optimism(h) => format!("optimism:{h}"),
            #[cfg(feature = "base")]
            FoundryHardfork::Base(h) => format!("base:{h}"),
            FoundryHardfork::Tempo(h) => format!("tempo:{h}"),
            #[cfg(feature = "monad")]
            FoundryHardfork::Monad(h) => format!("monad:{h}"),
        }
    }
}

impl<'de> Deserialize<'de> for FoundryHardfork {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).map_err(serde::de::Error::custom)
    }
}

impl FromStr for FoundryHardfork {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let raw = s.trim();

        let Some((ns, fork_raw)) = raw.split_once(':') else {
            return EthereumHardfork::from_str(raw)
                .map(Self::Ethereum)
                .map_err(|_| format!("unknown ethereum hardfork '{raw}'"));
        };

        let ns = ns.trim().to_ascii_lowercase();
        let fork = fork_raw.trim().to_ascii_lowercase().replace(['-', ' '], "_");

        match ns.as_str() {
            "eth" | "ethereum" => EthereumHardfork::from_str(&fork)
                .map(Self::Ethereum)
                .map_err(|_| format!("unknown ethereum hardfork '{fork_raw}'")),

            #[cfg(feature = "optimism")]
            "op" | "optimism" => OpHardfork::from_str(&fork)
                .map(Self::Optimism)
                .map_err(|_| format!("unknown optimism hardfork '{fork_raw}'")),

            #[cfg(feature = "base")]
            "base" => BaseUpgrade::from_str(&fork)
                .map(Self::Base)
                .map_err(|_| format!("unknown base hardfork '{fork_raw}'")),

            "t" | "tempo" => TempoHardfork::from_str(&fork)
                .map(Self::Tempo)
                .map_err(|_| format!("unknown tempo hardfork '{fork_raw}'")),

            #[cfg(feature = "monad")]
            "m" | "monad" => MonadHardfork::from_str(&fork)
                .map(Self::Monad)
                .map_err(|_| format!("unknown monad hardfork '{fork_raw}'")),
            _ => EthereumHardfork::from_str(&fork)
                .map(Self::Ethereum)
                .map_err(|_| format!("unknown hardfork '{raw}'")),
        }
    }
}

impl FoundryHardfork {
    pub const fn ethereum(h: EthereumHardfork) -> Self {
        Self::Ethereum(h)
    }

    #[cfg(feature = "optimism")]
    pub const fn optimism(h: OpHardfork) -> Self {
        Self::Optimism(h)
    }

    #[cfg(feature = "base")]
    pub const fn base(h: BaseUpgrade) -> Self {
        Self::Base(h)
    }

    pub const fn tempo(h: TempoHardfork) -> Self {
        Self::Tempo(h)
    }

    #[cfg(feature = "monad")]
    pub const fn monad(h: MonadHardfork) -> Self {
        Self::Monad(h)
    }

    /// Returns the hardfork name without a network namespace prefix.
    pub fn name(&self) -> String {
        match self {
            Self::Ethereum(h) => format!("{h}"),
            #[cfg(feature = "optimism")]
            Self::Optimism(h) => format!("{h}"),
            #[cfg(feature = "base")]
            Self::Base(h) => format!("{h}"),
            Self::Tempo(h) => format!("{h}"),
            #[cfg(feature = "monad")]
            Self::Monad(h) => format!("{h}"),
        }
    }

    /// Returns the network namespace for this hardfork, or `None` for plain Ethereum.
    ///
    /// Mirrors the namespace prefix used in the `"network:hardfork"` serialization format.
    pub const fn namespace(&self) -> Option<&'static str> {
        match self {
            Self::Ethereum(_) => None,
            #[cfg(feature = "optimism")]
            Self::Optimism(_) => Some("optimism"),
            #[cfg(feature = "base")]
            Self::Base(_) => Some("base"),
            Self::Tempo(_) => Some("tempo"),
            #[cfg(feature = "monad")]
            Self::Monad(_) => Some("monad"),
        }
    }

    /// Auto-detect the active hardfork for a given chain at a specific timestamp.
    ///
    /// Tries Ethereum, then Optimism, Tempo, and Monad. Returns `None` for unknown chains.
    pub fn from_chain_and_timestamp(chain_id: u64, timestamp: u64) -> Option<Self> {
        let chain = Chain::from_id(chain_id);
        if let Some(fork) = EthereumHardfork::from_chain_and_timestamp(chain, timestamp) {
            return Some(Self::Ethereum(fork));
        }
        #[cfg(feature = "base")]
        if let Some(fork) = BaseUpgrade::from_chain_and_timestamp(chain_id, timestamp) {
            return Some(Self::Base(fork));
        }
        #[cfg(feature = "optimism")]
        if let Some(fork) = OpHardfork::from_chain_and_timestamp(chain, timestamp) {
            return Some(Self::Optimism(fork));
        }
        if let Some(fork) = TempoHardfork::from_chain_and_timestamp(chain_id, timestamp) {
            return Some(Self::Tempo(fork));
        }
        #[cfg(feature = "monad")]
        if let Some(fork) = MonadHardfork::from_chain_and_timestamp(chain_id, timestamp) {
            return Some(Self::Monad(fork));
        }
        None
    }
}

impl From<EthereumHardfork> for FoundryHardfork {
    fn from(value: EthereumHardfork) -> Self {
        Self::Ethereum(value)
    }
}

impl From<FoundryHardfork> for EthereumHardfork {
    fn from(fork: FoundryHardfork) -> Self {
        match fork {
            FoundryHardfork::Ethereum(hardfork) => hardfork,
            _ => Self::default(),
        }
    }
}

#[cfg(feature = "optimism")]
impl From<OpHardfork> for FoundryHardfork {
    fn from(value: OpHardfork) -> Self {
        Self::Optimism(value)
    }
}

#[cfg(feature = "optimism")]
impl From<FoundryHardfork> for OpHardfork {
    fn from(fork: FoundryHardfork) -> Self {
        match fork {
            FoundryHardfork::Optimism(hardfork) => hardfork,
            _ => Self::default(),
        }
    }
}

#[cfg(feature = "base")]
impl From<BaseUpgrade> for FoundryHardfork {
    fn from(value: BaseUpgrade) -> Self {
        Self::Base(value)
    }
}

#[cfg(feature = "base")]
impl From<FoundryHardfork> for BaseUpgrade {
    fn from(fork: FoundryHardfork) -> Self {
        match fork {
            FoundryHardfork::Base(upgrade) => upgrade,
            _ => Self::default(),
        }
    }
}

#[cfg(feature = "base")]
impl From<FoundryHardfork> for BaseSpecId {
    fn from(fork: FoundryHardfork) -> Self {
        match fork {
            FoundryHardfork::Base(upgrade) => Self::new(upgrade),
            _ => Self::default(),
        }
    }
}

impl From<TempoHardfork> for FoundryHardfork {
    fn from(value: TempoHardfork) -> Self {
        Self::Tempo(value)
    }
}

impl From<FoundryHardfork> for TempoHardfork {
    fn from(fork: FoundryHardfork) -> Self {
        match fork {
            FoundryHardfork::Tempo(hardfork) => hardfork,
            _ => Self::default(),
        }
    }
}

#[cfg(feature = "monad")]
impl From<MonadHardfork> for FoundryHardfork {
    fn from(value: MonadHardfork) -> Self {
        Self::Monad(value)
    }
}

#[cfg(feature = "monad")]
impl From<FoundryHardfork> for MonadHardfork {
    fn from(fork: FoundryHardfork) -> Self {
        match fork {
            FoundryHardfork::Monad(hardfork) => hardfork,
            _ => Self::default(),
        }
    }
}

impl From<FoundryHardfork> for SpecId {
    fn from(fork: FoundryHardfork) -> Self {
        match fork {
            FoundryHardfork::Ethereum(hardfork) => spec_id_from_ethereum_hardfork(hardfork),
            #[cfg(feature = "optimism")]
            FoundryHardfork::Optimism(hardfork) => eth_spec_id_from_optimism_hardfork(hardfork),
            #[cfg(feature = "base")]
            FoundryHardfork::Base(hardfork) => eth_spec_id_from_base_upgrade(hardfork),
            FoundryHardfork::Tempo(hardfork) => spec_id_from_tempo_hardfork(hardfork),
            #[cfg(feature = "monad")]
            FoundryHardfork::Monad(hardfork) => hardfork.into(),
        }
    }
}

#[cfg(feature = "optimism")]
impl From<FoundryHardfork> for OpSpecId {
    fn from(fork: FoundryHardfork) -> Self {
        match fork {
            FoundryHardfork::Optimism(hardfork) => spec_id_from_optimism_hardfork(hardfork),
            _ => Self::default(),
        }
    }
}

/// Map an `EthereumHardfork` enum into its corresponding `SpecId`.
pub fn spec_id_from_ethereum_hardfork(hardfork: EthereumHardfork) -> SpecId {
    match hardfork {
        EthereumHardfork::Frontier => SpecId::FRONTIER,
        EthereumHardfork::Homestead => SpecId::HOMESTEAD,
        EthereumHardfork::Dao => SpecId::HOMESTEAD,
        EthereumHardfork::Tangerine => SpecId::TANGERINE,
        EthereumHardfork::SpuriousDragon => SpecId::SPURIOUS_DRAGON,
        EthereumHardfork::Byzantium => SpecId::BYZANTIUM,
        EthereumHardfork::Constantinople => SpecId::PETERSBURG,
        EthereumHardfork::Petersburg => SpecId::PETERSBURG,
        EthereumHardfork::Istanbul => SpecId::ISTANBUL,
        EthereumHardfork::MuirGlacier => SpecId::ISTANBUL,
        EthereumHardfork::Berlin => SpecId::BERLIN,
        EthereumHardfork::London => SpecId::LONDON,
        EthereumHardfork::ArrowGlacier => SpecId::LONDON,
        EthereumHardfork::GrayGlacier => SpecId::LONDON,
        EthereumHardfork::Paris => SpecId::MERGE,
        EthereumHardfork::Shanghai => SpecId::SHANGHAI,
        EthereumHardfork::Cancun => SpecId::CANCUN,
        EthereumHardfork::Prague => SpecId::PRAGUE,
        EthereumHardfork::Osaka => SpecId::OSAKA,
        EthereumHardfork::Bpo1 | EthereumHardfork::Bpo2 => SpecId::OSAKA,
        EthereumHardfork::Bpo3 | EthereumHardfork::Bpo4 | EthereumHardfork::Bpo5 => {
            unimplemented!()
        }
        EthereumHardfork::Amsterdam => SpecId::AMSTERDAM,
        f => unreachable!("unimplemented {}", f),
    }
}

/// Map an `OptimismHardfork` enum into its corresponding `OpSpecId`.
#[cfg(feature = "optimism")]
pub fn spec_id_from_optimism_hardfork(hardfork: OpHardfork) -> OpSpecId {
    match hardfork {
        OpHardfork::Bedrock => OpSpecId::BEDROCK,
        OpHardfork::Regolith => OpSpecId::REGOLITH,
        OpHardfork::Canyon => OpSpecId::CANYON,
        OpHardfork::Ecotone => OpSpecId::ECOTONE,
        OpHardfork::Fjord => OpSpecId::FJORD,
        OpHardfork::Granite => OpSpecId::GRANITE,
        OpHardfork::Holocene => OpSpecId::HOLOCENE,
        OpHardfork::Isthmus => OpSpecId::ISTHMUS,
        OpHardfork::Jovian => OpSpecId::JOVIAN,
        OpHardfork::Karst => OpSpecId::KARST,
        OpHardfork::Lagoon => OpSpecId::LAGOON,
        f => unreachable!("unimplemented {}", f),
    }
}

/// Map an `OptimismHardfork` enum into its corresponding Ethereum `SpecId`.
#[cfg(feature = "optimism")]
pub fn eth_spec_id_from_optimism_hardfork(hardfork: OpHardfork) -> SpecId {
    match hardfork {
        OpHardfork::Bedrock | OpHardfork::Regolith => SpecId::MERGE,
        OpHardfork::Canyon => SpecId::SHANGHAI,
        OpHardfork::Ecotone | OpHardfork::Fjord | OpHardfork::Granite | OpHardfork::Holocene => {
            SpecId::CANCUN
        }
        OpHardfork::Isthmus | OpHardfork::Jovian => SpecId::PRAGUE,
        OpHardfork::Karst | OpHardfork::Lagoon => SpecId::OSAKA,
        f => unreachable!("unimplemented {}", f),
    }
}

#[cfg(feature = "base")]
pub fn eth_spec_id_from_base_upgrade(hardfork: BaseUpgrade) -> SpecId {
    match hardfork {
        BaseUpgrade::Bedrock | BaseUpgrade::Regolith => SpecId::MERGE,
        BaseUpgrade::Canyon | BaseUpgrade::Delta => SpecId::SHANGHAI,
        BaseUpgrade::Ecotone
        | BaseUpgrade::Fjord
        | BaseUpgrade::Granite
        | BaseUpgrade::Holocene
        | BaseUpgrade::PectraBlobSchedule => SpecId::CANCUN,
        BaseUpgrade::Isthmus | BaseUpgrade::Jovian => SpecId::PRAGUE,
        BaseUpgrade::Azul | BaseUpgrade::Beryl | BaseUpgrade::Cobalt => SpecId::OSAKA,
        f => unreachable!("unimplemented {}", f),
    }
}

/// Map a `TempoHardfork` enum into its corresponding Ethereum `SpecId`.
pub const fn spec_id_from_tempo_hardfork(_: TempoHardfork) -> SpecId {
    SpecId::OSAKA
}

/// Trait for converting an [`EvmVersion`] into a network-specific spec type.
pub trait FromEvmVersion: From<FoundryHardfork> {
    fn from_evm_version(version: EvmVersion) -> Self;
}

/// Trait for parsing and displaying a network-specific execution spec.
pub trait ExecutionSpec: FromEvmVersion {
    // Returns the user-facing name for the active execution spec.
    fn evm_version_name(&self) -> String;

    // Parses an unnamespaced hardfork name for the active network.
    fn from_network_hardfork(_: &str) -> Option<Self> {
        None
    }

    // Converts a namespaced Foundry hardfork if it belongs to this spec family.
    fn from_foundry_hardfork(hardfork: FoundryHardfork) -> Option<Self>;
}

impl FromEvmVersion for SpecId {
    fn from_evm_version(version: EvmVersion) -> Self {
        match version {
            EvmVersion::Homestead => Self::HOMESTEAD,
            EvmVersion::TangerineWhistle => Self::TANGERINE,
            EvmVersion::SpuriousDragon => Self::SPURIOUS_DRAGON,
            EvmVersion::Byzantium => Self::BYZANTIUM,
            EvmVersion::Constantinople => Self::PETERSBURG,
            EvmVersion::Petersburg => Self::PETERSBURG,
            EvmVersion::Istanbul => Self::ISTANBUL,
            EvmVersion::Berlin => Self::BERLIN,
            EvmVersion::London => Self::LONDON,
            EvmVersion::Paris => Self::MERGE,
            EvmVersion::Shanghai => Self::SHANGHAI,
            EvmVersion::Cancun => Self::CANCUN,
            EvmVersion::Prague => Self::PRAGUE,
            EvmVersion::Osaka => Self::OSAKA,
            EvmVersion::Amsterdam => Self::AMSTERDAM,
        }
    }
}

impl ExecutionSpec for SpecId {
    // Returns the user-facing name for the active execution spec.
    fn evm_version_name(&self) -> String {
        self.to_string()
    }

    // Parses an unnamespaced Ethereum hardfork name.
    fn from_network_hardfork(hardfork: &str) -> Option<Self> {
        EthereumHardfork::from_str(hardfork).ok().map(spec_id_from_ethereum_hardfork)
    }

    // Converts only Ethereum namespaced hardforks to an Ethereum spec.
    fn from_foundry_hardfork(hardfork: FoundryHardfork) -> Option<Self> {
        match hardfork {
            FoundryHardfork::Ethereum(hardfork) => Some(spec_id_from_ethereum_hardfork(hardfork)),
            _ => None,
        }
    }
}

#[cfg(feature = "optimism")]
impl FromEvmVersion for OpSpecId {
    fn from_evm_version(version: EvmVersion) -> Self {
        match version {
            EvmVersion::Homestead
            | EvmVersion::TangerineWhistle
            | EvmVersion::SpuriousDragon
            | EvmVersion::Byzantium
            | EvmVersion::Constantinople
            | EvmVersion::Petersburg
            | EvmVersion::Istanbul
            | EvmVersion::Berlin
            | EvmVersion::London
            | EvmVersion::Paris => Self::BEDROCK,
            EvmVersion::Shanghai => Self::CANYON,
            EvmVersion::Cancun => Self::ECOTONE,
            EvmVersion::Prague => Self::ISTHMUS,
            EvmVersion::Osaka | EvmVersion::Amsterdam => Self::KARST,
        }
    }
}

#[cfg(feature = "optimism")]
impl ExecutionSpec for OpSpecId {
    // Returns the user-facing name for the active execution spec.
    fn evm_version_name(&self) -> String {
        let name: &'static str = (*self).into();
        name.to_string()
    }

    // Parses an unnamespaced Optimism hardfork name.
    fn from_network_hardfork(hardfork: &str) -> Option<Self> {
        OpHardfork::from_str(hardfork).ok().map(spec_id_from_optimism_hardfork)
    }

    // Converts only Optimism namespaced hardforks to an Optimism spec.
    fn from_foundry_hardfork(hardfork: FoundryHardfork) -> Option<Self> {
        match hardfork {
            FoundryHardfork::Optimism(hardfork) => Some(spec_id_from_optimism_hardfork(hardfork)),
            _ => None,
        }
    }
}

impl FromEvmVersion for TempoHardfork {
    fn from_evm_version(_: EvmVersion) -> Self {
        latest_active_tempo_hardfork()
    }
}

impl ExecutionSpec for TempoHardfork {
    // Returns the user-facing name for the active execution spec.
    fn evm_version_name(&self) -> String {
        self.to_string()
    }

    // Parses an unnamespaced Tempo hardfork name.
    fn from_network_hardfork(hardfork: &str) -> Option<Self> {
        Self::from_str(hardfork).ok()
    }

    // Converts only Tempo namespaced hardforks to a Tempo spec.
    fn from_foundry_hardfork(hardfork: FoundryHardfork) -> Option<Self> {
        match hardfork {
            FoundryHardfork::Tempo(hardfork) => Some(hardfork),
            _ => None,
        }
    }
}

#[cfg(feature = "monad")]
impl FromEvmVersion for MonadHardfork {
    fn from_evm_version(_: EvmVersion) -> Self {
        Self::default()
    }
}

#[cfg(feature = "monad")]
impl ExecutionSpec for MonadHardfork {
    // Returns the user-facing name for the active execution spec.
    fn evm_version_name(&self) -> String {
        self.to_string()
    }

    // Parses an unnamespaced Monad hardfork name.
    fn from_network_hardfork(hardfork: &str) -> Option<Self> {
        Self::from_str(hardfork).ok()
    }

    // Converts only Monad namespaced hardforks to a Monad spec.
    fn from_foundry_hardfork(hardfork: FoundryHardfork) -> Option<Self> {
        match hardfork {
            #[cfg(feature = "monad")]
            FoundryHardfork::Monad(hardfork) => Some(hardfork),
            _ => None,
        }
    }
}

#[cfg(feature = "base")]
impl FromEvmVersion for BaseSpecId {
    fn from_evm_version(version: EvmVersion) -> Self {
        let upgrade = match version {
            EvmVersion::Homestead
            | EvmVersion::TangerineWhistle
            | EvmVersion::SpuriousDragon
            | EvmVersion::Byzantium
            | EvmVersion::Constantinople
            | EvmVersion::Petersburg
            | EvmVersion::Istanbul
            | EvmVersion::Berlin
            | EvmVersion::London
            | EvmVersion::Paris => BaseUpgrade::Bedrock,
            EvmVersion::Shanghai => BaseUpgrade::Canyon,
            EvmVersion::Cancun => BaseUpgrade::Ecotone,
            EvmVersion::Prague => BaseUpgrade::Isthmus,
            EvmVersion::Osaka | EvmVersion::Amsterdam => BaseUpgrade::Azul,
        };
        Self::new(upgrade)
    }
}

#[cfg(feature = "base")]
impl ExecutionSpec for BaseSpecId {
    // Returns the user-facing name for the active execution spec.
    fn evm_version_name(&self) -> String {
        self.to_string()
    }

    // Parses an unnamespaced Base hardfork name.
    fn from_network_hardfork(hardfork: &str) -> Option<Self> {
        Self::from_str(hardfork).ok()
    }

    // Converts only Base namespaced hardforks to a Base spec.
    fn from_foundry_hardfork(hardfork: FoundryHardfork) -> Option<Self> {
        match hardfork {
            FoundryHardfork::Base(hardfork) => Some(Self::new(hardfork)),
            _ => None,
        }
    }
}

/// Returns the spec id derived from [`EvmVersion`] for a given spec type.
pub fn evm_spec_id<SPEC: FromEvmVersion>(evm_version: EvmVersion) -> SPEC {
    SPEC::from_evm_version(evm_version)
}

/// Returns the latest Tempo hardfork that has an activation on a known Tempo network.
pub fn latest_active_tempo_hardfork() -> TempoHardfork {
    // Tempo currently publishes activation timestamps through chain-aware hardfork resolution.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(u64::MAX);
    TempoHardfork::from_chain_and_timestamp(4217, now)
        .or_else(|| TempoHardfork::from_chain_and_timestamp(42431, now))
        .unwrap_or_default()
}

// Parses an EVM version or network-specific hardfork into the given spec type.
pub fn evm_spec_id_from_str<SPEC: ExecutionSpec>(evm_version: &str) -> Option<SPEC> {
    let evm_version = evm_version.trim();

    if let Ok(version) = EvmVersion::from_str(evm_version) {
        return Some(evm_spec_id(version));
    }

    if let Some(spec) = SPEC::from_network_hardfork(evm_version) {
        return Some(spec);
    }

    FoundryHardfork::from_str(evm_version).ok().and_then(SPEC::from_foundry_hardfork)
}

/// Convert a `BlockNumberOrTag` into an `EthereumHardfork`.
pub fn ethereum_hardfork_from_block_tag(block: impl Into<BlockNumberOrTag>) -> EthereumHardfork {
    let num = match block.into() {
        BlockNumberOrTag::Earliest => 0,
        BlockNumberOrTag::Number(num) => num,
        _ => u64::MAX,
    };

    EthereumHardfork::from_mainnet_block_number(num)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_hardforks::ethereum::mainnet::*;
    use tempo_hardfork::constants::{mainnet::*, moderato::*};

    #[test]
    fn test_ethereum_spec_id_mapping() {
        assert_eq!(spec_id_from_ethereum_hardfork(EthereumHardfork::Frontier), SpecId::FRONTIER);
        assert_eq!(spec_id_from_ethereum_hardfork(EthereumHardfork::Homestead), SpecId::HOMESTEAD);

        // Test latest hardforks
        assert_eq!(spec_id_from_ethereum_hardfork(EthereumHardfork::Cancun), SpecId::CANCUN);
        assert_eq!(spec_id_from_ethereum_hardfork(EthereumHardfork::Prague), SpecId::PRAGUE);
        assert_eq!(spec_id_from_ethereum_hardfork(EthereumHardfork::Osaka), SpecId::OSAKA);
        assert_eq!(spec_id_from_ethereum_hardfork(EthereumHardfork::Amsterdam), SpecId::AMSTERDAM);
    }

    #[test]
    fn test_tempo_spec_id_mapping() {
        assert_eq!(spec_id_from_tempo_hardfork(TempoHardfork::Genesis), SpecId::OSAKA);
        assert_eq!(spec_id_from_tempo_hardfork(TempoHardfork::T8), SpecId::OSAKA);
    }

    #[test]
    fn test_tempo_evm_version_defaults_to_latest_active_hardfork() {
        let latest = latest_active_tempo_hardfork();
        assert_eq!(evm_spec_id::<TempoHardfork>(EvmVersion::Osaka), latest);
    }

    #[test]
    #[cfg(feature = "monad")]
    fn test_monad_hardfork_parsing() {
        assert_eq!(
            "monad:MonadNine".parse::<FoundryHardfork>().unwrap(),
            FoundryHardfork::Monad(MonadHardfork::MonadNine)
        );
        assert_eq!(
            "monad:MonadTen".parse::<FoundryHardfork>().unwrap(),
            FoundryHardfork::Monad(MonadHardfork::MonadTen)
        );
        assert_eq!(
            "m:MonadNext".parse::<FoundryHardfork>().unwrap(),
            FoundryHardfork::Monad(MonadHardfork::MonadNext)
        );
    }

    #[test]
    #[cfg(feature = "monad")]
    fn test_monad_hardfork_serialization() {
        assert_eq!(
            String::from(FoundryHardfork::Monad(MonadHardfork::MonadEight)),
            "monad:MonadEight"
        );
        assert_eq!(FoundryHardfork::Monad(MonadHardfork::MonadEight).namespace(), Some("monad"));
        assert_eq!(FoundryHardfork::Monad(MonadHardfork::MonadEight).name(), "MonadEight");
    }

    #[test]
    #[cfg(feature = "monad")]
    fn test_monad_hardfork_spec_id_mapping() {
        assert_eq!(SpecId::from(FoundryHardfork::Monad(MonadHardfork::MonadEight)), SpecId::PRAGUE);
        assert_eq!(SpecId::from(FoundryHardfork::Monad(MonadHardfork::MonadNine)), SpecId::OSAKA);
        assert_eq!(SpecId::from(FoundryHardfork::Monad(MonadHardfork::MonadTen)), SpecId::OSAKA);
        assert_eq!(
            MonadHardfork::from(FoundryHardfork::Monad(MonadHardfork::MonadNext)),
            MonadHardfork::MonadNext
        );
    }

    #[test]
    fn test_tempo_hardfork_from_chain_and_timestamp() {
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(4217, u64::MAX),
            Some(FoundryHardfork::Tempo(TempoHardfork::T10))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(42431, u64::MAX),
            Some(FoundryHardfork::Tempo(TempoHardfork::T10))
        );

        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(MAINNET_CHAIN_ID, MAINNET_T6_TIMESTAMP - 1),
            Some(FoundryHardfork::Tempo(TempoHardfork::T5))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(MAINNET_CHAIN_ID, MAINNET_T6_TIMESTAMP),
            Some(FoundryHardfork::Tempo(TempoHardfork::T6))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(MAINNET_CHAIN_ID, MAINNET_T7_TIMESTAMP - 1),
            Some(FoundryHardfork::Tempo(TempoHardfork::T6))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(MAINNET_CHAIN_ID, MAINNET_T7_TIMESTAMP),
            Some(FoundryHardfork::Tempo(TempoHardfork::T7))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(MAINNET_CHAIN_ID, MAINNET_T8_TIMESTAMP - 1),
            Some(FoundryHardfork::Tempo(TempoHardfork::T7))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(MAINNET_CHAIN_ID, MAINNET_T8_TIMESTAMP),
            Some(FoundryHardfork::Tempo(TempoHardfork::T8))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(MAINNET_CHAIN_ID, MAINNET_T9_TIMESTAMP - 1),
            Some(FoundryHardfork::Tempo(TempoHardfork::T8))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(MAINNET_CHAIN_ID, MAINNET_T9_TIMESTAMP),
            Some(FoundryHardfork::Tempo(TempoHardfork::T9))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(MODERATO_CHAIN_ID, MODERATO_T6_TIMESTAMP - 1),
            Some(FoundryHardfork::Tempo(TempoHardfork::T5))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(MODERATO_CHAIN_ID, MODERATO_T6_TIMESTAMP),
            Some(FoundryHardfork::Tempo(TempoHardfork::T6))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(MODERATO_CHAIN_ID, MODERATO_T7_TIMESTAMP - 1),
            Some(FoundryHardfork::Tempo(TempoHardfork::T6))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(MODERATO_CHAIN_ID, MODERATO_T7_TIMESTAMP),
            Some(FoundryHardfork::Tempo(TempoHardfork::T7))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(MODERATO_CHAIN_ID, MODERATO_T8_TIMESTAMP - 1),
            Some(FoundryHardfork::Tempo(TempoHardfork::T7))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(MODERATO_CHAIN_ID, MODERATO_T8_TIMESTAMP),
            Some(FoundryHardfork::Tempo(TempoHardfork::T8))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(MODERATO_CHAIN_ID, MODERATO_T9_TIMESTAMP - 1),
            Some(FoundryHardfork::Tempo(TempoHardfork::T8))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(MODERATO_CHAIN_ID, MODERATO_T9_TIMESTAMP),
            Some(FoundryHardfork::Tempo(TempoHardfork::T9))
        );
    }

    #[test]
    #[cfg(feature = "monad")]
    fn test_monad_hardfork_from_chain_and_timestamp() {
        let mainnet_ten = MonadHardfork::MonadTen.mainnet_activation_timestamp().unwrap();
        let testnet_ten = MonadHardfork::MonadTen.testnet_activation_timestamp().unwrap();

        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(
                monad_revm::MONAD_MAINNET_CHAIN_ID,
                1_763_648_999,
            ),
            Some(FoundryHardfork::Monad(MonadHardfork::MonadEight))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(
                monad_revm::MONAD_MAINNET_CHAIN_ID,
                1_773_930_600,
            ),
            Some(FoundryHardfork::Monad(MonadHardfork::MonadNine))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(
                monad_revm::MONAD_MAINNET_CHAIN_ID,
                mainnet_ten - 1,
            ),
            Some(FoundryHardfork::Monad(MonadHardfork::MonadNine))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(
                monad_revm::MONAD_MAINNET_CHAIN_ID,
                mainnet_ten,
            ),
            Some(FoundryHardfork::Monad(MonadHardfork::MonadTen))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(
                monad_revm::MONAD_TESTNET_CHAIN_ID,
                1_773_152_999,
            ),
            Some(FoundryHardfork::Monad(MonadHardfork::MonadEight))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(
                monad_revm::MONAD_TESTNET_CHAIN_ID,
                1_773_153_000,
            ),
            Some(FoundryHardfork::Monad(MonadHardfork::MonadNine))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(
                monad_revm::MONAD_TESTNET_CHAIN_ID,
                testnet_ten - 1,
            ),
            Some(FoundryHardfork::Monad(MonadHardfork::MonadNine))
        );
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(
                monad_revm::MONAD_TESTNET_CHAIN_ID,
                testnet_ten,
            ),
            Some(FoundryHardfork::Monad(MonadHardfork::MonadTen))
        );
    }

    #[test]
    fn test_evm_spec_id_from_str_parses_network_hardforks() {
        assert_eq!(evm_spec_id_from_str::<TempoHardfork>("T3"), Some(TempoHardfork::T3));
        assert_eq!(evm_spec_id_from_str::<TempoHardfork>("tempo:T2"), Some(TempoHardfork::T2));
        assert_eq!(evm_spec_id_from_str::<TempoHardfork>("tempo:T7"), Some(TempoHardfork::T7));
        assert_eq!(evm_spec_id_from_str::<TempoHardfork>("tempo:T8"), Some(TempoHardfork::T8));
        assert_eq!(evm_spec_id_from_str::<TempoHardfork>("ethereum:prague"), None);

        #[cfg(feature = "monad")]
        {
            assert_eq!(
                evm_spec_id_from_str::<MonadHardfork>("MonadNine"),
                Some(MonadHardfork::MonadNine)
            );
            assert_eq!(
                evm_spec_id_from_str::<MonadHardfork>("MonadTen"),
                Some(MonadHardfork::MonadTen)
            );
            assert_eq!(
                evm_spec_id_from_str::<MonadHardfork>("monad:MonadEight"),
                Some(MonadHardfork::MonadEight)
            );
            assert_eq!(evm_spec_id_from_str::<MonadHardfork>("tempo:T3"), None);
        }
    }

    #[test]
    fn test_hardfork_from_block_tag_numbers() {
        assert_eq!(
            ethereum_hardfork_from_block_tag(MAINNET_HOMESTEAD_BLOCK - 1),
            EthereumHardfork::Frontier
        );
        assert_eq!(
            ethereum_hardfork_from_block_tag(MAINNET_LONDON_BLOCK + 1),
            EthereumHardfork::London
        );
    }

    #[test]
    fn test_from_chain_and_timestamp_ethereum_mainnet() {
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(1, 0),
            Some(FoundryHardfork::Ethereum(EthereumHardfork::Frontier))
        );
        // Shanghai activated at timestamp 1681338455 on mainnet
        assert_eq!(
            FoundryHardfork::from_chain_and_timestamp(1, 1_681_338_455),
            Some(FoundryHardfork::Ethereum(EthereumHardfork::Shanghai))
        );
    }

    #[test]
    fn test_from_chain_and_timestamp_sepolia() {
        let sepolia_chain_id = 11155111;
        assert!(FoundryHardfork::from_chain_and_timestamp(sepolia_chain_id, u64::MAX).is_some());
    }

    #[test]
    fn test_from_chain_and_timestamp_unknown_chain() {
        assert_eq!(FoundryHardfork::from_chain_and_timestamp(999999, 0), None);
    }

    #[cfg(feature = "base")]
    mod base {
        use super::*;
        use base_common_genesis::UpgradeConfig;

        #[test]
        fn test_base_hardfork_serialization() {
            assert_eq!(String::from(FoundryHardfork::Base(BaseUpgrade::Azul)), "base:Azul");
            assert_eq!(FoundryHardfork::Base(BaseUpgrade::Azul).namespace(), Some("base"));
            assert_eq!(FoundryHardfork::Base(BaseUpgrade::Azul).name(), "Azul");
        }

        #[test]
        fn test_base_hardfork_spec_id_mapping() {
            assert_eq!(SpecId::from(FoundryHardfork::Base(BaseUpgrade::Azul)), SpecId::OSAKA);
            assert_eq!(SpecId::from(FoundryHardfork::Base(BaseUpgrade::Jovian)), SpecId::PRAGUE);
            assert_eq!(
                BaseSpecId::from(FoundryHardfork::Base(BaseUpgrade::Azul)),
                BaseSpecId::new(BaseUpgrade::Azul)
            );
        }

        #[test]
        fn test_base_hardfork_parsing() {
            assert_eq!(
                "base:Azul".parse::<FoundryHardfork>().unwrap(),
                FoundryHardfork::Base(BaseUpgrade::Azul)
            );
            assert_eq!(
                "base:Beryl".parse::<FoundryHardfork>().unwrap(),
                FoundryHardfork::Base(BaseUpgrade::Beryl)
            );
        }

        #[test]
        fn test_base_hardfork_from_chain_and_timestamp() {
            let mainnet_config = UpgradeConfig::BASE_MAINNET;
            let sepolia_config = UpgradeConfig::BASE_SEPOLIA;
            assert_eq!(
                FoundryHardfork::from_chain_and_timestamp(8453, u64::MAX),
                Some(FoundryHardfork::Base(BaseUpgrade::Beryl))
            );
            assert_eq!(
                FoundryHardfork::from_chain_and_timestamp(84532, u64::MAX),
                Some(FoundryHardfork::Base(BaseUpgrade::Beryl))
            );
            assert_eq!(
                FoundryHardfork::from_chain_and_timestamp(8453, mainnet_config.base.azul.unwrap()),
                Some(FoundryHardfork::Base(BaseUpgrade::Azul))
            );
            assert_eq!(
                FoundryHardfork::from_chain_and_timestamp(84532, sepolia_config.base.azul.unwrap()),
                Some(FoundryHardfork::Base(BaseUpgrade::Azul))
            );
            assert_eq!(
                FoundryHardfork::from_chain_and_timestamp(
                    8453,
                    mainnet_config.base.azul.unwrap() - 1
                ),
                Some(FoundryHardfork::Base(BaseUpgrade::Jovian))
            );
            assert_eq!(
                FoundryHardfork::from_chain_and_timestamp(
                    84532,
                    sepolia_config.base.azul.unwrap() - 1
                ),
                Some(FoundryHardfork::Base(BaseUpgrade::Jovian))
            );
        }

        #[test]
        fn test_evm_spec_id_from_str_parses_base_upgrades() {
            assert_eq!(
                evm_spec_id_from_str::<BaseSpecId>("Azul"),
                Some(BaseSpecId::new(BaseUpgrade::Azul))
            );
            assert_eq!(
                evm_spec_id_from_str::<BaseSpecId>("base:Beryl"),
                Some(BaseSpecId::new(BaseUpgrade::Beryl))
            );
            assert_eq!(evm_spec_id_from_str::<BaseSpecId>("tempo:T3"), None);
        }
    }

    #[cfg(feature = "optimism")]
    mod optimism {
        use super::*;

        #[test]
        fn test_optimism_spec_id_mapping() {
            assert_eq!(spec_id_from_optimism_hardfork(OpHardfork::Bedrock), OpSpecId::BEDROCK);
            assert_eq!(spec_id_from_optimism_hardfork(OpHardfork::Regolith), OpSpecId::REGOLITH);

            // Test latest hardforks
            assert_eq!(spec_id_from_optimism_hardfork(OpHardfork::Holocene), OpSpecId::HOLOCENE);
            assert_eq!(spec_id_from_optimism_hardfork(OpHardfork::Karst), OpSpecId::KARST);
            assert_eq!(spec_id_from_optimism_hardfork(OpHardfork::Lagoon), OpSpecId::LAGOON);
            assert_eq!(eth_spec_id_from_optimism_hardfork(OpHardfork::Jovian), SpecId::PRAGUE);
            assert_eq!(eth_spec_id_from_optimism_hardfork(OpHardfork::Karst), SpecId::OSAKA);
            assert_eq!(eth_spec_id_from_optimism_hardfork(OpHardfork::Lagoon), SpecId::OSAKA);
            assert_eq!(evm_spec_id::<OpSpecId>(EvmVersion::Osaka), OpSpecId::KARST);
        }

        #[test]
        fn test_from_chain_and_timestamp_op_mainnet() {
            let op_chain_id = 10;
            assert!(matches!(
                FoundryHardfork::from_chain_and_timestamp(op_chain_id, u64::MAX),
                Some(FoundryHardfork::Optimism(_))
            ));
        }

        /// Base is an OP-stack chain, so without the `base` feature its chain IDs must still map
        /// to an Optimism hardfork rather than resolving to nothing.
        #[test]
        #[cfg(not(feature = "base"))]
        fn test_from_chain_and_timestamp_base() {
            let base_chain_id = 8453;
            assert!(matches!(
                FoundryHardfork::from_chain_and_timestamp(base_chain_id, u64::MAX),
                Some(FoundryHardfork::Optimism(_))
            ));
        }
    }
}
