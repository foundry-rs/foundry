//! `cast` subcommands.
//!
//! All subcommands should respect the `foundry_config::Config`.
//! If a subcommand accepts values that are supported by the `Config`, then the subcommand should
//! implement `figment::Provider` which allows the subcommand to override the config's defaults, see
//! [`foundry_config::Config`].

use alloy_network::AnyNetwork;
use alloy_provider::Provider;
use eyre::Result;
use foundry_cli::utils::load_config_from_provider;
use foundry_common::provider::ProviderBuilder;
use foundry_config::{Config, figment::Figment};
use foundry_evm::opts::EvmOpts;
use foundry_evm_networks::NetworkVariant;

/// Loads Cast's config and applies its normalized network to the EVM options.
pub(crate) fn load_cast_config_and_evm_opts(figment: Figment) -> Result<(Box<Config>, EvmOpts)> {
    let config = Box::new(load_config_from_provider(figment.clone())?);
    let mut evm_opts = figment.extract::<EvmOpts>()?;
    evm_opts.networks = config.networks;
    Ok((config, evm_opts))
}

pub mod access_list;
pub mod artifact;
mod auth;
pub mod b2e_payload;
pub mod batch_mktx;
pub mod batch_send;
pub mod bind;
pub mod call;
pub mod call_overrides;
pub mod constructor_args;
pub mod create2;
pub mod creation_code;
#[cfg(any(feature = "base", feature = "optimism"))]
pub mod da_estimate;
pub mod erc20;
pub mod estimate;
pub mod find_block;
pub mod interface;
pub mod keychain;
pub mod logs;
pub(crate) mod miner;
pub mod mktx;
pub mod receive_policy;
pub mod rpc;
pub mod run;
pub mod send;
pub mod storage;
pub mod storage_credits;
pub mod tempo;
pub(crate) mod tempo_policy_args;
pub mod tip20;
pub mod tip403;
pub mod trace;
pub mod txpool;
pub mod vaddr;
pub mod wallet;

/// Resolves the configured network, falling back to the RPC chain ID.
pub(crate) async fn resolve_network(config: &Config) -> eyre::Result<NetworkVariant> {
    if let Some(network) = config.networks.resolved_network() {
        return Ok(network);
    }
    if let Some(chain) = config.chain {
        return Ok(chain.id().into());
    }

    let provider = ProviderBuilder::<AnyNetwork>::from_config(config)?.build()?;
    Ok(provider.get_chain_id().await?.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "monad")]
    #[test]
    fn normalized_hardfork_network_is_applied_to_evm_opts() {
        let figment = Config::figment().merge(("hardfork", "monad:MonadNine"));
        let (config, evm_opts) = load_cast_config_and_evm_opts(figment).unwrap();

        assert!(config.networks.is_monad());
        assert!(evm_opts.networks.is_monad());
    }

    #[cfg(feature = "base")]
    #[tokio::test]
    async fn resolve_network_preserves_explicit_base() {
        let config = Config { networks: NetworkVariant::Base.into(), ..Default::default() };
        assert_eq!(resolve_network(&config).await.unwrap(), NetworkVariant::Base);
    }
}
