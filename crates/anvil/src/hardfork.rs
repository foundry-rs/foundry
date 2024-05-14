use alloy_rpc_types::BlockNumberOrTag;
use foundry_evm::revm::primitives::SpecId;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Hardfork {
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
    #[default]
    Latest,
}

impl Hardfork {
    /// Get the first block number of the hardfork.
    pub fn fork_block(&self) -> u64 {
        match *self {
            Hardfork::Frontier => 0,
            Hardfork::Homestead => 1150000,
            Hardfork::Dao => 1920000,
            Hardfork::Tangerine => 2463000,
            Hardfork::SpuriousDragon => 2675000,
            Hardfork::Byzantium => 4370000,
            Hardfork::Constantinople | Hardfork::Petersburg => 7280000,
            Hardfork::Istanbul => 9069000,
            Hardfork::Muirglacier => 9200000,
            Hardfork::Berlin => 12244000,
            Hardfork::London => 12965000,
            Hardfork::ArrowGlacier => 13773000,
            Hardfork::GrayGlacier => 15050000,
            Hardfork::Paris => 15537394,
            Hardfork::Shanghai => 17034870,
            Hardfork::Cancun | Hardfork::Latest => 19426587,
        }
    }
}

impl FromStr for Hardfork {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_lowercase();
        let hardfork = match s.as_str() {
            "frontier" | "1" => Hardfork::Frontier,
            "homestead" | "2" => Hardfork::Homestead,
            "dao" | "3" => Hardfork::Dao,
            "tangerine" | "4" => Hardfork::Tangerine,
            "spuriousdragon" | "5" => Hardfork::SpuriousDragon,
            "byzantium" | "6" => Hardfork::Byzantium,
            "constantinople" | "7" => Hardfork::Constantinople,
            "petersburg" | "8" => Hardfork::Petersburg,
            "istanbul" | "9" => Hardfork::Istanbul,
            "muirglacier" | "10" => Hardfork::Muirglacier,
            "berlin" | "11" => Hardfork::Berlin,
            "london" | "12" => Hardfork::London,
            "arrowglacier" | "13" => Hardfork::ArrowGlacier,
            "grayglacier" | "14" => Hardfork::GrayGlacier,
            "paris" | "merge" | "15" => Hardfork::Paris,
            "shanghai" | "16" => Hardfork::Shanghai,
            "cancun" | "17" => Hardfork::Cancun,
            "latest" => Hardfork::Latest,
            _ => return Err(format!("Unknown hardfork {s}")),
        };
        Ok(hardfork)
    }
}

impl From<Hardfork> for SpecId {
    fn from(fork: Hardfork) -> Self {
        match fork {
            Hardfork::Frontier => SpecId::FRONTIER,
            Hardfork::Homestead => SpecId::HOMESTEAD,
            Hardfork::Dao => SpecId::HOMESTEAD,
            Hardfork::Tangerine => SpecId::TANGERINE,
            Hardfork::SpuriousDragon => SpecId::SPURIOUS_DRAGON,
            Hardfork::Byzantium => SpecId::BYZANTIUM,
            Hardfork::Constantinople => SpecId::CONSTANTINOPLE,
            Hardfork::Petersburg => SpecId::PETERSBURG,
            Hardfork::Istanbul => SpecId::ISTANBUL,
            Hardfork::Muirglacier => SpecId::MUIR_GLACIER,
            Hardfork::Berlin => SpecId::BERLIN,
            Hardfork::London => SpecId::LONDON,
            Hardfork::ArrowGlacier => SpecId::LONDON,
            Hardfork::GrayGlacier => SpecId::GRAY_GLACIER,
            Hardfork::Paris => SpecId::MERGE,
            Hardfork::Shanghai => SpecId::SHANGHAI,
            Hardfork::Cancun | Hardfork::Latest => SpecId::CANCUN,
        }
    }
}

impl<T: Into<BlockNumberOrTag>> From<T> for Hardfork {
    fn from(block: T) -> Self {
        let num = match block.into() {
            BlockNumberOrTag::Earliest => 0,
            BlockNumberOrTag::Number(num) => num,
            _ => u64::MAX,
        };

        match num {
            _i if num < 1_150_000 => Hardfork::Frontier,
            _i if num < 1_920_000 => Hardfork::Dao,
            _i if num < 2_463_000 => Hardfork::Homestead,
            _i if num < 2_675_000 => Hardfork::Tangerine,
            _i if num < 4_370_000 => Hardfork::SpuriousDragon,
            _i if num < 7_280_000 => Hardfork::Byzantium,
            _i if num < 9_069_000 => Hardfork::Constantinople,
            _i if num < 9_200_000 => Hardfork::Istanbul,
            _i if num < 12_244_000 => Hardfork::Muirglacier,
            _i if num < 12_965_000 => Hardfork::Berlin,
            _i if num < 13_773_000 => Hardfork::London,
            _i if num < 15_050_000 => Hardfork::ArrowGlacier,
            _i if num < 17_034_870 => Hardfork::Paris,
            _i if num < 19_426_587 => Hardfork::Shanghai,
            _ => Hardfork::Latest,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Hardfork;

    #[test]
    fn test_hardfork_blocks() {
        let hf: Hardfork = 12_965_000u64.into();
        assert_eq!(hf, Hardfork::London);

        let hf: Hardfork = 4370000u64.into();
        assert_eq!(hf, Hardfork::Byzantium);

        let hf: Hardfork = 12244000u64.into();
        assert_eq!(hf, Hardfork::Berlin);
    }
}
