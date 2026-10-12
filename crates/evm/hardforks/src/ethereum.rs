//! Ethereum hardfork selection for evm2 execution.

use crate::EthereumHardfork;
use evm2::SpecId;
use foundry_compilers::artifacts::EvmVersion;

/// Maps an Ethereum hardfork to its evm2 execution spec.
pub fn ethereum_spec_id(hardfork: EthereumHardfork) -> SpecId {
    match hardfork {
        EthereumHardfork::Frontier => SpecId::FRONTIER,
        EthereumHardfork::Homestead | EthereumHardfork::Dao => SpecId::HOMESTEAD,
        EthereumHardfork::Tangerine => SpecId::TANGERINE,
        EthereumHardfork::SpuriousDragon => SpecId::SPURIOUS_DRAGON,
        EthereumHardfork::Byzantium => SpecId::BYZANTIUM,
        EthereumHardfork::Constantinople | EthereumHardfork::Petersburg => SpecId::PETERSBURG,
        EthereumHardfork::Istanbul | EthereumHardfork::MuirGlacier => SpecId::ISTANBUL,
        EthereumHardfork::Berlin => SpecId::BERLIN,
        EthereumHardfork::London
        | EthereumHardfork::ArrowGlacier
        | EthereumHardfork::GrayGlacier => SpecId::LONDON,
        EthereumHardfork::Paris => SpecId::MERGE,
        EthereumHardfork::Shanghai => SpecId::SHANGHAI,
        EthereumHardfork::Cancun => SpecId::CANCUN,
        EthereumHardfork::Prague => SpecId::PRAGUE,
        EthereumHardfork::Osaka | EthereumHardfork::Bpo1 | EthereumHardfork::Bpo2 => SpecId::OSAKA,
        EthereumHardfork::Bpo3 | EthereumHardfork::Bpo4 | EthereumHardfork::Bpo5 => SpecId::OSAKA,
        EthereumHardfork::Amsterdam => SpecId::AMSTERDAM,
        fork => unreachable!("unimplemented {fork}"),
    }
}

/// Maps the configured compiler EVM version to its evm2 execution spec.
pub const fn ethereum_spec_from_evm_version(version: EvmVersion) -> SpecId {
    match version {
        EvmVersion::Homestead => SpecId::HOMESTEAD,
        EvmVersion::TangerineWhistle => SpecId::TANGERINE,
        EvmVersion::SpuriousDragon => SpecId::SPURIOUS_DRAGON,
        EvmVersion::Byzantium => SpecId::BYZANTIUM,
        EvmVersion::Constantinople | EvmVersion::Petersburg => SpecId::PETERSBURG,
        EvmVersion::Istanbul => SpecId::ISTANBUL,
        EvmVersion::Berlin => SpecId::BERLIN,
        EvmVersion::London => SpecId::LONDON,
        EvmVersion::Paris => SpecId::MERGE,
        EvmVersion::Shanghai => SpecId::SHANGHAI,
        EvmVersion::Cancun => SpecId::CANCUN,
        EvmVersion::Prague => SpecId::PRAGUE,
        EvmVersion::Osaka => SpecId::OSAKA,
        EvmVersion::Amsterdam => SpecId::AMSTERDAM,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_selection_preserves_ethereum_aliases() {
        for (hardfork, spec) in [
            (EthereumHardfork::Dao, SpecId::HOMESTEAD),
            (EthereumHardfork::Constantinople, SpecId::PETERSBURG),
            (EthereumHardfork::MuirGlacier, SpecId::ISTANBUL),
            (EthereumHardfork::ArrowGlacier, SpecId::LONDON),
            (EthereumHardfork::GrayGlacier, SpecId::LONDON),
            (EthereumHardfork::Paris, SpecId::MERGE),
            (EthereumHardfork::Bpo1, SpecId::OSAKA),
            (EthereumHardfork::Bpo2, SpecId::OSAKA),
        ] {
            assert_eq!(ethereum_spec_id(hardfork), spec);
        }
        assert_eq!(ethereum_spec_from_evm_version(EvmVersion::Constantinople), SpecId::PETERSBURG);
    }
}
