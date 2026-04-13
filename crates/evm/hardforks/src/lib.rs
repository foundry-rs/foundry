//! EVM hardfork definitions for Foundry.
//!
//! Provides [`FoundryHardfork`], a unified enum over Ethereum, Optimism, and Tempo hardforks
//! with `FromStr`/`Serialize`/`Deserialize` support for CLI and config usage.

use std::str::FromStr;

use alloy_chains::Chain;
use alloy_rpc_types::BlockNumberOrTag;
use foundry_compilers::artifacts::EvmVersion;
use op_revm::OpSpecId;
use revm::primitives::hardfork::SpecId;
use serde::{Deserialize, Serialize};

pub use alloy_hardforks::EthereumHardfork;
pub use alloy_op_hardforks::OpHardfork;
pub use tempo_chainspec::hardfork::TempoHardfork;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(into = "String")]
pub enum FoundryHardfork {
    Ethereum(EthereumHardfork),
    Optimism(OpHardfork),
    Tempo(TempoHardfork),
}

impl From<FoundryHardfork> for String {
    fn from(fork: FoundryHardfork) -> Self {
        match fork {
            FoundryHardfork::Ethereum(h) => format!("{h}"),
            FoundryHardfork::Optimism(h) => format!("optimism:{h}"),
            FoundryHardfork::Tempo(h) => format!("tempo:{h}"),
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

            "op" | "optimism" => OpHardfork::from_str(&fork)
                .map(Self::Optimism)
                .map_err(|_| format!("unknown optimism hardfork '{fork_raw}'")),

            "t" | "tempo" => TempoHardfork::from_str(&fork)
                .map(Self::Tempo)
                .map_err(|_| format!("unknown tempo hardfork '{fork_raw}'")),
            _ => EthereumHardfork::from_str(&fork)
                .map(Self::Ethereum)
                .map_err(|_| format!("unknown hardfork '{raw}'")),
        }
    }
}

impl FoundryHardfork {
    pub fn ethereum(h: EthereumHardfork) -> Self {
        Self::Ethereum(h)
    }

    pub fn optimism(h: OpHardfork) -> Self {
        Self::Optimism(h)
    }

    pub fn tempo(h: TempoHardfork) -> Self {
        Self::Tempo(h)
    }

    /// Returns the hardfork name without a network namespace prefix.
    pub fn name(&self) -> String {
        match self {
            Self::Ethereum(h) => format!("{h}"),
            Self::Optimism(h) => format!("{h}"),
            Self::Tempo(h) => format!("{h}"),
        }
    }

    /// Auto-detect the active hardfork for a given chain at a specific timestamp.
    ///
    /// Tries Ethereum, then Optimism. Returns `None` for unknown chains.
    pub fn from_chain_and_timestamp(chain_id: u64, timestamp: u64) -> Option<Self> {
        let chain = Chain::from_id(chain_id);
        if let Some(fork) = EthereumHardfork::from_chain_and_timestamp(chain, timestamp) {
            return Some(Self::Ethereum(fork));
        }
        if let Some(fork) = OpHardfork::from_chain_and_timestamp(chain, timestamp) {
            return Some(Self::Optimism(fork));
        }
        // TODO: add tempo support after https://github.com/tempoxyz/tempo/pull/3514 release
        // providing TempoHardfork::from_chain_and_timestamp
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

impl From<OpHardfork> for FoundryHardfork {
    fn from(value: OpHardfork) -> Self {
        Self::Optimism(value)
    }
}

impl From<FoundryHardfork> for OpHardfork {
    fn from(fork: FoundryHardfork) -> Self {
        match fork {
            FoundryHardfork::Optimism(hardfork) => hardfork,
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

impl From<FoundryHardfork> for SpecId {
    fn from(fork: FoundryHardfork) -> Self {
        match fork {
            FoundryHardfork::Ethereum(hardfork) => spec_id_from_ethereum_hardfork(hardfork),
            FoundryHardfork::Optimism(hardfork) => spec_id_from_optimism_hardfork(hardfork).into(),
            FoundryHardfork::Tempo(hardfork) => hardfork.into(),
        }
    }
}

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
        EthereumHardfork::Dao => SpecId::DAO_FORK,
        EthereumHardfork::Tangerine => SpecId::TANGERINE,
        EthereumHardfork::SpuriousDragon => SpecId::SPURIOUS_DRAGON,
        EthereumHardfork::Byzantium => SpecId::BYZANTIUM,
        EthereumHardfork::Constantinople => SpecId::CONSTANTINOPLE,
        EthereumHardfork::Petersburg => SpecId::PETERSBURG,
        EthereumHardfork::Istanbul => SpecId::ISTANBUL,
        EthereumHardfork::MuirGlacier => SpecId::MUIR_GLACIER,
        EthereumHardfork::Berlin => SpecId::BERLIN,
        EthereumHardfork::London => SpecId::LONDON,
        EthereumHardfork::ArrowGlacier => SpecId::ARROW_GLACIER,
        EthereumHardfork::GrayGlacier => SpecId::GRAY_GLACIER,
        EthereumHardfork::Paris => SpecId::MERGE,
        EthereumHardfork::Shanghai => SpecId::SHANGHAI,
        EthereumHardfork::Cancun => SpecId::CANCUN,
        EthereumHardfork::Prague => SpecId::PRAGUE,
        EthereumHardfork::Osaka => SpecId::OSAKA,
        EthereumHardfork::Bpo1 | EthereumHardfork::Bpo2 => SpecId::OSAKA,
        EthereumHardfork::Bpo3 | EthereumHardfork::Bpo4 | EthereumHardfork::Bpo5 => {
            unimplemented!()
        }
        f => unreachable!("unimplemented {}", f),
    }
}

/// Map an `OptimismHardfork` enum into its corresponding `OpSpecId`.
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
        OpHardfork::Interop => OpSpecId::INTEROP,
        OpHardfork::Jovian => OpSpecId::JOVIAN,
        f => unreachable!("unimplemented {}", f),
    }
}

/// Trait for converting an [`EvmVersion`] into a network-specific spec type.
pub trait FromEvmVersion: From<FoundryHardfork> {
    fn from_evm_version(version: EvmVersion) -> Self;
}

impl FromEvmVersion for SpecId {
    fn from_evm_version(version: EvmVersion) -> Self {
        match version {
            EvmVersion::Homestead => Self::HOMESTEAD,
            EvmVersion::TangerineWhistle => Self::TANGERINE,
            EvmVersion::SpuriousDragon => Self::SPURIOUS_DRAGON,
            EvmVersion::Byzantium => Self::BYZANTIUM,
            EvmVersion::Constantinople => Self::CONSTANTINOPLE,
            EvmVersion::Petersburg => Self::PETERSBURG,
            EvmVersion::Istanbul => Self::ISTANBUL,
            EvmVersion::Berlin => Self::BERLIN,
            EvmVersion::London => Self::LONDON,
            EvmVersion::Paris => Self::MERGE,
            EvmVersion::Shanghai => Self::SHANGHAI,
            EvmVersion::Cancun => Self::CANCUN,
            EvmVersion::Prague => Self::PRAGUE,
            EvmVersion::Osaka => Self::OSAKA,
        }
    }
}

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
            EvmVersion::Osaka => Self::JOVIAN,
        }
    }
}

impl FromEvmVersion for TempoHardfork {
    fn from_evm_version(_: EvmVersion) -> Self {
        Self::default()
    }
}

/// Returns the spec id derived from [`EvmVersion`] for a given spec type.
pub fn evm_spec_id<SPEC: FromEvmVersion>(evm_version: EvmVersion) -> SPEC {
    SPEC::from_evm_version(evm_version)
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

    #[test]
    fn test_ethereum_spec_id_mapping() {
        assert_eq!(spec_id_from_ethereum_hardfork(EthereumHardfork::Frontier), SpecId::FRONTIER);
        assert_eq!(spec_id_from_ethereum_hardfork(EthereumHardfork::Homestead), SpecId::HOMESTEAD);

        // Test latest hardforks
        assert_eq!(spec_id_from_ethereum_hardfork(EthereumHardfork::Cancun), SpecId::CANCUN);
        assert_eq!(spec_id_from_ethereum_hardfork(EthereumHardfork::Prague), SpecId::PRAGUE);
        assert_eq!(spec_id_from_ethereum_hardfork(EthereumHardfork::Osaka), SpecId::OSAKA);
    }

    #[test]
    fn test_optimism_spec_id_mapping() {
        assert_eq!(spec_id_from_optimism_hardfork(OpHardfork::Bedrock), OpSpecId::BEDROCK);
        assert_eq!(spec_id_from_optimism_hardfork(OpHardfork::Regolith), OpSpecId::REGOLITH);

        // Test latest hardforks
        assert_eq!(spec_id_from_optimism_hardfork(OpHardfork::Holocene), OpSpecId::HOLOCENE);
        assert_eq!(spec_id_from_optimism_hardfork(OpHardfork::Interop), OpSpecId::INTEROP);
    }

    #[test]
    fn test_tempo_spec_id_mapping() {
        assert_eq!(SpecId::from(TempoHardfork::Genesis), SpecId::OSAKA);
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
    fn test_from_chain_and_timestamp_op_mainnet() {
        let op_chain_id = 10;
        assert!(matches!(
            FoundryHardfork::from_chain_and_timestamp(op_chain_id, u64::MAX),
            Some(FoundryHardfork::Optimism(_))
        ));
    }

    #[test]
    fn test_from_chain_and_timestamp_base() {
        let base_chain_id = 8453;
        assert!(matches!(
            FoundryHardfork::from_chain_and_timestamp(base_chain_id, u64::MAX),
            Some(FoundryHardfork::Optimism(_))
        ));
    }

    #[test]
    fn test_from_chain_and_timestamp_unknown_chain() {
        assert_eq!(FoundryHardfork::from_chain_and_timestamp(999999, 0), None);
    }
}
