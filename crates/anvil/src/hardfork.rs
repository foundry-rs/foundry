use alloy_hardforks::EthereumHardfork;
use alloy_op_hardforks::OpHardfork::{self};
use alloy_rpc_types::BlockNumberOrTag;

use op_revm::OpSpecId;
use revm::primitives::hardfork::SpecId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChainHardfork {
    Ethereum(EthereumHardfork),
    Optimism(OpHardfork),
}

impl From<EthereumHardfork> for ChainHardfork {
    fn from(value: EthereumHardfork) -> Self {
        Self::Ethereum(value)
    }
}

impl From<OpHardfork> for ChainHardfork {
    fn from(value: OpHardfork) -> Self {
        Self::Optimism(value)
    }
}

impl From<ChainHardfork> for SpecId {
    fn from(fork: ChainHardfork) -> Self {
        match fork {
            ChainHardfork::Ethereum(hardfork) => spec_id_from_ethereum_hardfork(hardfork),
            ChainHardfork::Optimism(hardfork) => spec_id_from_optimism_hardfork(hardfork).into(),
        }
    }
}

/// Map an EthereumHardfork enum into its corresponding SpecId.
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
    }
}

/// Map an OptimismHardfork enum into its corresponding OpSpecId.
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
        OpHardfork::Jovian => OpSpecId::ISTHMUS,
    }
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
    #[allow(unused_imports)]
    use alloy_rpc_types::BlockNumberOrTag;

    #[test]
    fn test_ethereum_spec_id_mapping() {
        assert_eq!(spec_id_from_ethereum_hardfork(EthereumHardfork::Frontier), SpecId::FRONTIER);
        assert_eq!(spec_id_from_ethereum_hardfork(EthereumHardfork::Homestead), SpecId::HOMESTEAD);

        // Test latest hardforks
        assert_eq!(spec_id_from_ethereum_hardfork(EthereumHardfork::Cancun), SpecId::CANCUN);
        assert_eq!(spec_id_from_ethereum_hardfork(EthereumHardfork::Prague), SpecId::PRAGUE);
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
}
