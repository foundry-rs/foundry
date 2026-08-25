//! `cast` subcommands.
//!
//! All subcommands should respect the `foundry_config::Config`.
//! If a subcommand accepts values that are supported by the `Config`, then the subcommand should
//! implement `figment::Provider` which allows the subcommand to override the config's defaults, see
//! [`foundry_config::Config`].

use eyre::Result;
use foundry_cli::utils::load_config_from_provider;
use foundry_config::{Config, figment::Figment};
use foundry_evm::opts::EvmOpts;

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
#[cfg(feature = "optimism")]
pub mod da_estimate;
pub mod erc20;
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

#[cfg(all(test, feature = "monad"))]
mod tests {
    use super::*;

    #[test]
    fn normalized_hardfork_network_is_applied_to_evm_opts() {
        let figment = Config::figment().merge(("hardfork", "monad:MonadNine"));
        let (config, evm_opts) = load_cast_config_and_evm_opts(figment).unwrap();

        assert!(config.networks.is_monad());
        assert!(evm_opts.networks.is_monad());
    }
}
