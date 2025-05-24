use std::str::FromStr;

use alloy_hardforks::ethereum::mainnet::*;
use alloy_rpc_types::BlockNumberOrTag;
use eyre::bail;
use op_revm::OpSpecId;
use revm::primitives::hardfork::SpecId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChainHardfork {
    Ethereum(EthereumHardfork),
    Optimism(OptimismHardfork),
}

impl From<EthereumHardfork> for ChainHardfork {
    fn from(value: EthereumHardfork) -> Self {
        Self::Ethereum(value)
    }
}

impl From<OptimismHardfork> for ChainHardfork {
    fn from(value: OptimismHardfork) -> Self {
        Self::Optimism(value)
    }
}

impl From<ChainHardfork> for SpecId {
    fn from(fork: ChainHardfork) -> Self {
        match fork {
            ChainHardfork::Ethereum(hardfork) => hardfork.into(),
            ChainHardfork::Optimism(hardfork) => hardfork.into_eth_spec(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EthereumHardfork {
    Frontier,
    Homestead,
    Dao,
    Tangerine,
    SpuriousDragon,
    Byzantium,
    Constantinople,
    Petersburg,
    Istanbul,
    Muirglacier,
    Berlin,
    London,
    ArrowGlacier,
    GrayGlacier,
    Paris,
    Shanghai,
    Cancun,
    Prague,
    #[default]
    Latest,
}

impl EthereumHardfork {
    /// Get the first block number of the hardfork.
    pub fn fork_block(&self) -> u64 {
        match *self {
            Self::Frontier => MAINNET_FRONTIER_BLOCK,
            Self::Homestead => MAINNET_HOMESTEAD_BLOCK,
            Self::Dao => MAINNET_DAO_BLOCK,
            Self::Tangerine => MAINNET_TANGERINE_BLOCK,
            Self::SpuriousDragon => MAINNET_SPURIOUS_DRAGON_BLOCK,
            Self::Byzantium => MAINNET_BYZANTIUM_BLOCK,
            Self::Constantinople => MAINNET_CONSTANTINOPLE_BLOCK,
            Self::Petersburg => MAINNET_PETERSBURG_BLOCK,
            Self::Istanbul => MAINNET_ISTANBUL_BLOCK,
            Self::Muirglacier => MAINNET_MUIR_GLACIER_BLOCK,
            Self::Berlin => MAINNET_BERLIN_BLOCK,
            Self::London => MAINNET_LONDON_BLOCK,
            Self::ArrowGlacier => MAINNET_ARROW_GLACIER_BLOCK,
            Self::GrayGlacier => MAINNET_GRAY_GLACIER_BLOCK,
            Self::Paris => MAINNET_PARIS_BLOCK,
            Self::Shanghai => MAINNET_SHANGHAI_BLOCK,
            Self::Cancun | Self::Latest => MAINNET_CANCUN_BLOCK,
            Self::Prague => MAINNET_PRAGUE_BLOCK,
        }
    }
    pub fn from_block_number(block: u64) -> Option<Self> {
        Some(match block {
            _i if block < MAINNET_HOMESTEAD_BLOCK => Self::Frontier,
            _i if block < MAINNET_DAO_BLOCK => Self::Homestead,
            _i if block < MAINNET_TANGERINE_BLOCK => Self::Dao,
            _i if block < MAINNET_SPURIOUS_DRAGON_BLOCK => Self::Tangerine,
            _i if block < MAINNET_BYZANTIUM_BLOCK => Self::SpuriousDragon,
            _i if block < MAINNET_CONSTANTINOPLE_BLOCK => Self::Byzantium,
            _i if block < MAINNET_ISTANBUL_BLOCK => Self::Constantinople,
            _i if block < MAINNET_MUIR_GLACIER_BLOCK => Self::Istanbul,
            _i if block < MAINNET_BERLIN_BLOCK => Self::Muirglacier,
            _i if block < MAINNET_LONDON_BLOCK => Self::Berlin,
            _i if block < MAINNET_ARROW_GLACIER_BLOCK => Self::London,
            _i if block < MAINNET_PARIS_BLOCK => Self::ArrowGlacier,
            _i if block < MAINNET_SHANGHAI_BLOCK => Self::Paris,
            _i if block < MAINNET_CANCUN_BLOCK => Self::Shanghai,
            _i if block < MAINNET_PRAGUE_BLOCK => Self::Cancun,
            _i if block <= u64::MAX => Self::Prague,
            _ => return None,
        })
    }
}

impl FromStr for EthereumHardfork {
    type Err = eyre::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_lowercase();
        let hardfork = match s.as_str() {
            "frontier" | "1" => Self::Frontier,
            "homestead" | "2" => Self::Homestead,
            "dao" | "3" => Self::Dao,
            "tangerine" | "4" => Self::Tangerine,
            "spuriousdragon" | "5" => Self::SpuriousDragon,
            "byzantium" | "6" => Self::Byzantium,
            "constantinople" | "7" => Self::Constantinople,
            "petersburg" | "8" => Self::Petersburg,
            "istanbul" | "9" => Self::Istanbul,
            "muirglacier" | "10" => Self::Muirglacier,
            "berlin" | "11" => Self::Berlin,
            "london" | "12" => Self::London,
            "arrowglacier" | "13" => Self::ArrowGlacier,
            "grayglacier" | "14" => Self::GrayGlacier,
            "paris" | "merge" | "15" => Self::Paris,
            "shanghai" | "16" => Self::Shanghai,
            "cancun" | "17" => Self::Cancun,
            "prague" | "18" => Self::Prague,
            "latest" => Self::Latest,
            _ => bail!("Unknown hardfork {s}"),
        };
        Ok(hardfork)
    }
}

impl From<EthereumHardfork> for SpecId {
    fn from(fork: EthereumHardfork) -> Self {
        match fork {
            EthereumHardfork::Frontier => Self::FRONTIER,
            EthereumHardfork::Homestead => Self::HOMESTEAD,
            EthereumHardfork::Dao => Self::HOMESTEAD,
            EthereumHardfork::Tangerine => Self::TANGERINE,
            EthereumHardfork::SpuriousDragon => Self::SPURIOUS_DRAGON,
            EthereumHardfork::Byzantium => Self::BYZANTIUM,
            EthereumHardfork::Constantinople => Self::CONSTANTINOPLE,
            EthereumHardfork::Petersburg => Self::PETERSBURG,
            EthereumHardfork::Istanbul => Self::ISTANBUL,
            EthereumHardfork::Muirglacier => Self::MUIR_GLACIER,
            EthereumHardfork::Berlin => Self::BERLIN,
            EthereumHardfork::London => Self::LONDON,
            EthereumHardfork::ArrowGlacier => Self::LONDON,
            EthereumHardfork::GrayGlacier => Self::GRAY_GLACIER,
            EthereumHardfork::Paris => Self::MERGE,
            EthereumHardfork::Shanghai => Self::SHANGHAI,
            EthereumHardfork::Cancun | EthereumHardfork::Latest => Self::CANCUN,
            EthereumHardfork::Prague => Self::PRAGUE,
        }
    }
}

impl<T: Into<BlockNumberOrTag>> From<T> for EthereumHardfork {
    fn from(block: T) -> Self {
        let num = match block.into() {
            BlockNumberOrTag::Earliest => 0,
            BlockNumberOrTag::Number(num) => num,
            _ => u64::MAX,
        };

        match num {
            _i if num < MAINNET_HOMESTEAD_BLOCK => Self::Frontier,
            _i if num < MAINNET_DAO_BLOCK => Self::Homestead,
            _i if num < MAINNET_TANGERINE_BLOCK => Self::Dao,
            _i if num < MAINNET_SPURIOUS_DRAGON_BLOCK => Self::Tangerine,
            _i if num < MAINNET_BYZANTIUM_BLOCK => Self::SpuriousDragon,
            _i if num < MAINNET_CONSTANTINOPLE_BLOCK => Self::Byzantium,
            _i if num < MAINNET_ISTANBUL_BLOCK => Self::Constantinople,
            _i if num < MAINNET_MUIR_GLACIER_BLOCK => Self::Istanbul,
            _i if num < MAINNET_BERLIN_BLOCK => Self::Muirglacier,
            _i if num < MAINNET_LONDON_BLOCK => Self::Berlin,
            _i if num < MAINNET_ARROW_GLACIER_BLOCK => Self::London,
            _i if num < MAINNET_PARIS_BLOCK => Self::ArrowGlacier,
            _i if num < MAINNET_SHANGHAI_BLOCK => Self::Paris,
            _i if num < MAINNET_CANCUN_BLOCK => Self::Shanghai,
            _ => Self::Latest,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OptimismHardfork {
    Bedrock,
    Regolith,
    Canyon,
    Ecotone,
    Fjord,
    Granite,
    Holocene,
    #[default]
    Isthmus,
}

impl OptimismHardfork {
    pub fn into_eth_spec(self) -> SpecId {
        let op_spec: OpSpecId = self.into();
        op_spec.into_eth_spec()
    }
}

impl FromStr for OptimismHardfork {
    type Err = eyre::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_lowercase();
        let hardfork = match s.as_str() {
            "bedrock" => Self::Bedrock,
            "regolith" => Self::Regolith,
            "canyon" => Self::Canyon,
            "ecotone" => Self::Ecotone,
            "fjord" => Self::Fjord,
            "granite" => Self::Granite,
            "holocene" => Self::Holocene,
            "isthmus" => Self::Isthmus,
            _ => bail!("Unknown hardfork {s}"),
        };
        Ok(hardfork)
    }
}

impl From<OptimismHardfork> for OpSpecId {
    fn from(fork: OptimismHardfork) -> Self {
        match fork {
            OptimismHardfork::Bedrock => Self::BEDROCK,
            OptimismHardfork::Regolith => Self::REGOLITH,
            OptimismHardfork::Canyon => Self::CANYON,
            OptimismHardfork::Ecotone => Self::ECOTONE,
            OptimismHardfork::Fjord => Self::FJORD,
            OptimismHardfork::Granite => Self::GRANITE,
            OptimismHardfork::Holocene => Self::HOLOCENE,
            OptimismHardfork::Isthmus => Self::ISTHMUS,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::EthereumHardfork;

    #[test]
    fn test_hardfork_blocks() {
        let hf: EthereumHardfork = 12_965_000u64.into();
        assert_eq!(hf, EthereumHardfork::London);

        let hf: EthereumHardfork = 4370000u64.into();
        assert_eq!(hf, EthereumHardfork::Byzantium);

        let hf: EthereumHardfork = 12244000u64.into();
        assert_eq!(hf, EthereumHardfork::Berlin);
    }
}
