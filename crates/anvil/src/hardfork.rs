use alloy_rpc_types::BlockNumberOrTag;
use eyre::bail;
use foundry_evm::revm::primitives::SpecId;
use std::str::FromStr;

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
            ChainHardfork::Optimism(hardfork) => hardfork.into(),
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
    PragueEOF,
    #[default]
    Latest,
}

impl EthereumHardfork {
    /// Get the first block number of the hardfork.
    pub fn fork_block(&self) -> u64 {
        match *self {
            Self::Frontier => 0,
            Self::Homestead => 1150000,
            Self::Dao => 1920000,
            Self::Tangerine => 2463000,
            Self::SpuriousDragon => 2675000,
            Self::Byzantium => 4370000,
            Self::Constantinople | Self::Petersburg => 7280000,
            Self::Istanbul => 9069000,
            Self::Muirglacier => 9200000,
            Self::Berlin => 12244000,
            Self::London => 12965000,
            Self::ArrowGlacier => 13773000,
            Self::GrayGlacier => 15050000,
            Self::Paris => 15537394,
            Self::Shanghai => 17034870,
            Self::Cancun | Self::Latest => 19426587,
            // TODO: add block after activation
            Self::Prague | Self::PragueEOF => unreachable!(),
        }
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
            "pragueeof" | "19" | "prague-eof" => Self::PragueEOF,
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
            // TODO: switch to latest after activation
            EthereumHardfork::PragueEOF => Self::PRAGUE_EOF,
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
            _i if num < 1_150_000 => Self::Frontier,
            _i if num < 1_920_000 => Self::Dao,
            _i if num < 2_463_000 => Self::Homestead,
            _i if num < 2_675_000 => Self::Tangerine,
            _i if num < 4_370_000 => Self::SpuriousDragon,
            _i if num < 7_280_000 => Self::Byzantium,
            _i if num < 9_069_000 => Self::Constantinople,
            _i if num < 9_200_000 => Self::Istanbul,
            _i if num < 12_244_000 => Self::Muirglacier,
            _i if num < 12_965_000 => Self::Berlin,
            _i if num < 13_773_000 => Self::London,
            _i if num < 15_050_000 => Self::ArrowGlacier,
            _i if num < 17_034_870 => Self::Paris,
            _i if num < 19_426_587 => Self::Shanghai,
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
    #[default]
    Latest,
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
            "latest" => Self::Latest,
            _ => bail!("Unknown hardfork {s}"),
        };
        Ok(hardfork)
    }
}

impl From<OptimismHardfork> for SpecId {
    fn from(fork: OptimismHardfork) -> Self {
        match fork {
            OptimismHardfork::Bedrock => Self::BEDROCK,
            OptimismHardfork::Regolith => Self::REGOLITH,
            OptimismHardfork::Canyon => Self::CANYON,
            OptimismHardfork::Ecotone => Self::ECOTONE,
            OptimismHardfork::Fjord => Self::FJORD,
            OptimismHardfork::Granite => Self::GRANITE,
            OptimismHardfork::Latest => Self::LATEST,
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
