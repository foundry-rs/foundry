//! Ethereum execution options resolved without a REVM environment.

use crate::opts::EvmOpts;
use alloy_primitives::U256;
use evm2::{EvmFeatures, SpecId, Version, env::BlockEnvExt};
use foundry_common::DEV_CHAIN_ID;
use foundry_config::Config;
use foundry_evm_hardforks::{FoundryHardfork, ethereum_spec_from_evm_version, ethereum_spec_id};
use foundry_evm_networks::NetworkVariant;

/// Configuration and block data for an Ethereum execution.
#[derive(Clone, Copy, Debug)]
pub struct EthereumEnv {
    pub spec: SpecId,
    pub version: Version,
    pub block: BlockEnvExt,
    /// Gas price observed by synthetic Foundry calls; transaction fee accounting stays at zero.
    pub gas_price: U256,
}

impl EthereumEnv {
    /// Uses the protocol defaults for `spec`.
    pub const fn new(spec: SpecId, block: BlockEnvExt) -> Self {
        Self { spec, version: Version::new(spec), block, gas_price: U256::ZERO }
    }

    /// Applies Foundry's local Ethereum execution options without a legacy EVM environment.
    pub fn local(spec: SpecId, opts: &EvmOpts) -> Self {
        let mut env = Self::new(
            spec,
            BlockEnvExt {
                number: opts.env.block_number,
                beneficiary: opts.env.block_coinbase,
                timestamp: opts.env.block_timestamp,
                gas_limit: U256::from(opts.gas_limit()),
                basefee: U256::from(opts.env.block_base_fee_per_gas),
                difficulty: U256::from(opts.env.block_difficulty),
                prevrandao: U256::from_be_slice(opts.env.block_prevrandao.as_slice()),
                ..Default::default()
            },
        );
        env.version.chain_id = opts.env.chain_id.unwrap_or(DEV_CHAIN_ID);
        env.gas_price = U256::from(opts.env.gas_price.unwrap_or_default());
        env.version.memory_limit = opts.memory_limit;
        env.version.max_code_size = opts.env.code_size_limit.unwrap_or(usize::MAX);
        env.version.max_initcode_size = env.version.max_code_size.saturating_mul(2);
        if !opts.enable_tx_gas_limit {
            env.version.tx_gas_limit_cap = u64::MAX;
        }
        env.version
            .features
            .remove(EvmFeatures::EIP3607 | EvmFeatures::NONCE_CHECK | EvmFeatures::BASE_FEE_CHECK);
        env.version.features.set(EvmFeatures::BLOCK_GAS_LIMIT_CHECK, !opts.disable_block_gas_limit);
        env
    }

    /// Selects the configured Ethereum hardfork and applies local execution options.
    pub fn local_from_config(config: &Config, opts: &EvmOpts) -> eyre::Result<Self> {
        eyre::ensure!(
            opts.networks.execution_network() == NetworkVariant::Ethereum,
            "Ethereum execution requires the Ethereum network"
        );
        let spec = match config.hardfork {
            Some(FoundryHardfork::Ethereum(hardfork)) => ethereum_spec_id(hardfork),
            Some(hardfork) => {
                eyre::bail!("{} is not an Ethereum hardfork", String::from(hardfork))
            }
            None => ethereum_spec_from_evm_version(config.evm_version),
        };
        Ok(Self::local(spec, opts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Address;
    use foundry_compilers::artifacts::EvmVersion;
    use foundry_evm_hardforks::{EthereumHardfork, TempoHardfork};

    #[test]
    fn local_environment_applies_foundry_execution_options() {
        let mut opts = EvmOpts::default();
        opts.env.chain_id = Some(31_337);
        opts.env.block_number = U256::from(9);
        opts.env.block_coinbase = Address::with_last_byte(0xa);
        opts.env.block_timestamp = U256::from(123);
        opts.env.block_base_fee_per_gas = 7;
        opts.env.gas_price = Some(9);
        opts.env.code_size_limit = Some(64_000);
        opts.memory_limit = 1_000_000;
        opts.disable_block_gas_limit = true;

        let env = EthereumEnv::local(SpecId::OSAKA, &opts);
        assert_eq!(env.block.number, U256::from(9));
        assert_eq!(env.block.beneficiary, opts.env.block_coinbase);
        assert_eq!(env.block.timestamp, U256::from(123));
        assert_eq!(env.block.basefee, U256::from(7));
        assert_eq!(env.block.gas_limit, U256::from(opts.gas_limit()));
        assert_eq!(env.gas_price, U256::from(9));
        assert_eq!(env.version.chain_id, 31_337);
        assert_eq!(env.version.memory_limit, 1_000_000);
        assert_eq!(env.version.max_code_size, 64_000);
        assert_eq!(env.version.max_initcode_size, 128_000);
        assert_eq!(env.version.tx_gas_limit_cap, u64::MAX);
        assert!(!env.version.feature(EvmFeatures::EIP3607));
        assert!(!env.version.feature(EvmFeatures::NONCE_CHECK));
        assert!(!env.version.feature(EvmFeatures::BLOCK_GAS_LIMIT_CHECK));
    }

    #[test]
    fn local_config_selects_only_ethereum_hardforks() {
        let mut config = Config { evm_version: EvmVersion::London, ..Default::default() };
        let opts = EvmOpts::default();
        assert_eq!(EthereumEnv::local_from_config(&config, &opts).unwrap().spec, SpecId::LONDON);

        config.hardfork = Some(FoundryHardfork::Ethereum(EthereumHardfork::Cancun));
        assert_eq!(EthereumEnv::local_from_config(&config, &opts).unwrap().spec, SpecId::CANCUN);

        config.hardfork = Some(FoundryHardfork::Tempo(TempoHardfork::T3));
        assert_eq!(
            EthereumEnv::local_from_config(&config, &opts).unwrap_err().to_string(),
            "tempo:T3 is not an Ethereum hardfork"
        );
    }
}
