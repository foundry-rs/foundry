//! `cast` subcommands.
//!
//! All subcommands should respect the `foundry_config::Config`.
//! If a subcommand accepts values that are supported by the `Config`, then the subcommand should
//! implement `figment::Provider` which allows the subcommand to override the config's defaults, see
//! [`foundry_config::Config`].

#[cfg(feature = "base")]
use alloy_network::AnyNetwork;
#[cfg(feature = "base")]
use alloy_provider::Provider;
use eyre::Result;
use foundry_cli::utils::load_config_from_provider;
#[cfg(feature = "base")]
use foundry_common::provider::ProviderBuilder;
use foundry_config::{Config, figment::Figment};
use foundry_evm::opts::EvmOpts;
#[cfg(feature = "base")]
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
pub mod erc4626;
pub mod estimate;
pub mod events;
pub mod find_block;
pub mod interface;
pub mod keychain;
pub mod logs;
pub(crate) mod miner;
pub mod mktx;
pub mod receive_policy;
pub mod rpc;
pub mod run;
pub mod safe;
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
///
/// Only Base-capable builds resolve a network here: every other family keeps picking its provider
/// from the flags it already reads, and paying for an extra `eth_chainId` on their behalf would
/// change behavior no other network asked for.
#[cfg(feature = "base")]
pub(crate) async fn resolve_network(config: &Config) -> eyre::Result<NetworkVariant> {
    if let Some(network) = config.networks.resolved_network() {
        return Ok(network);
    }
    if let Some(chain) = config.chain {
        return network_for_chain_id(chain.id());
    }

    let provider = ProviderBuilder::<AnyNetwork>::from_config(config)?.build()?;
    network_for_chain_id(provider.get_chain_id().await?)
}

/// Resolves a chain ID to its network family, reporting a disabled family as an error.
///
/// The infallible `From<ChainId>` conversion swallows that error and degrades to Ethereum, which
/// would make `cast tx` and `cast block --raw` disagree with `cast call` on the same input. Unknown
/// chain IDs still fall back to Ethereum, as before.
#[cfg(feature = "base")]
fn network_for_chain_id(chain_id: u64) -> eyre::Result<NetworkVariant> {
    NetworkVariant::from_known_chain_id(chain_id)
        .map_err(eyre::Report::msg)
        .map(|network| network.unwrap_or(NetworkVariant::Ethereum))
}

#[cfg(all(test, any(feature = "base", feature = "monad")))]
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

    #[cfg(feature = "base")]
    #[tokio::test]
    async fn resolve_network_infers_base_from_chain_id() {
        let config = Config {
            chain: Some(foundry_config::Chain::from_named(alloy_chains::NamedChain::Base)),
            ..Default::default()
        };
        assert_eq!(resolve_network(&config).await.unwrap(), NetworkVariant::Base);
    }

    /// A disabled family has to surface here too, otherwise `cast tx` reports Ethereum for input
    /// that `cast call` rejects.
    #[cfg(all(feature = "base", not(feature = "monad")))]
    #[tokio::test]
    async fn resolve_network_reports_disabled_family() {
        let config = Config {
            chain: Some(foundry_config::Chain::from_named(alloy_chains::NamedChain::Monad)),
            ..Default::default()
        };
        let err = resolve_network(&config).await.unwrap_err().to_string();
        assert!(err.contains("`monad` is not enabled"), "unexpected error: {err}");
    }

    #[cfg(feature = "base")]
    #[tokio::test]
    async fn resolve_network_still_defaults_unknown_chain_ids_to_ethereum() {
        let config =
            Config { chain: Some(foundry_config::Chain::from_id(u64::MAX)), ..Default::default() };
        assert_eq!(resolve_network(&config).await.unwrap(), NetworkVariant::Ethereum);
    }
}
