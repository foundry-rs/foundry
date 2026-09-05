//! `cast` subcommands.
//!
//! All subcommands should respect the `foundry_config::Config`.
//! If a subcommand accepts values that are supported by the `Config`, then the subcommand should
//! implement `figment::Provider` which allows the subcommand to override the config's defaults, see
//! [`foundry_config::Config`].

use alloy_network::Network;
use alloy_primitives::{Address, Bytes, map::AddressHashMap};
use alloy_provider::Provider;
use alloy_rpc_types::BlockId;
use eyre::Result;
use foundry_cli::{
    json::print_scalar,
    opts::RpcOpts,
    utils::{LoadConfig, get_provider, load_config_from_provider},
};
use foundry_common::{provider::RetryProvider, shell};
use foundry_config::{Config, figment::Figment};
use foundry_evm::{core::bytecode::InstIter, opts::EvmOpts};
use futures::StreamExt;
use serde::Serialize;
use serde_json::Value;
use std::fmt::{Display, Write};

const MAX_CONCURRENT_RPC_REQUESTS: usize = 5;

/// Loads Cast's config and applies its normalized network to the EVM options.
pub(crate) fn load_cast_config_and_evm_opts(figment: Figment) -> Result<(Box<Config>, EvmOpts)> {
    let config = Box::new(load_config_from_provider(figment.clone())?);
    let mut evm_opts = figment.extract::<EvmOpts>()?;
    evm_opts.networks = config.networks;
    Ok((config, evm_opts))
}

/// Loads the config for `rpc` and builds the default provider from it.
pub(crate) fn rpc_provider(rpc: &RpcOpts) -> Result<RetryProvider> {
    get_provider(&rpc.load_config()?)
}

/// Asks whether to continue after a warning, printing `Aborted.` when the user declines.
pub(crate) fn confirm_continue() -> Result<bool> {
    let response: String = foundry_common::prompt!("\nContinue anyway? [y/N] ")?;
    if matches!(response.trim(), "y" | "Y") {
        return Ok(true);
    }
    sh_status!("Aborted.")?;
    Ok(false)
}

/// Prints `json` pretty-printed (un-enveloped) in JSON mode, `plain` otherwise.
pub(crate) fn print_json_or(json: Value, plain: impl Display) -> Result<()> {
    if shell::is_json() {
        sh_println!("{}", serde_json::to_string_pretty(&json)?)?;
    } else {
        sh_println!("{plain}")?;
    }
    Ok(())
}

/// Prints the primary result of a command: a JSON envelope in `--json` mode, otherwise a raw line
/// that bypasses the shell verbosity layer so `--quiet` does not suppress it.
pub(crate) fn print_result_line(value: impl Serialize + Display) -> Result<()> {
    if shell::is_json() {
        return print_scalar(value);
    }
    print_raw_line(value)
}

/// Prints a raw line to stdout, bypassing the shell verbosity layer so `--quiet` does not
/// suppress it.
pub(crate) fn print_raw_line(value: impl Display) -> Result<()> {
    let mut shell = shell::Shell::get();
    let out = shell.out();
    writeln!(out, "{value}")?;
    out.flush()?;
    Ok(())
}

/// Fetches the non-empty code of `addresses` at `block` with bounded concurrency, warning about
/// addresses whose code cannot be fetched.
pub(crate) async fn fetch_code_via_rpc<N: Network, P: Provider<N>>(
    provider: &P,
    addresses: impl IntoIterator<Item = Address>,
    block: BlockId,
) -> AddressHashMap<Bytes> {
    let mut code_by_address = AddressHashMap::default();
    let mut requests = futures::stream::iter(addresses)
        .map(
            |address| async move { (address, provider.get_code_at(address).block_id(block).await) },
        )
        .buffer_unordered(MAX_CONCURRENT_RPC_REQUESTS);
    while let Some((address, code)) = requests.next().await {
        match code {
            Ok(code) if !code.is_empty() => {
                code_by_address.insert(address, code);
            }
            Ok(_) => {}
            Err(err) => {
                let _ = sh_warn!("Failed to fetch code for {address}: {err}");
            }
        }
    }
    code_by_address
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

pub(crate) fn disassemble(code: &[u8]) -> Result<String> {
    let mut output = String::new();
    for (pc, inst) in InstIter::new(code).with_pc() {
        writeln!(output, "{pc:08x}: {inst}")?;
    }
    Ok(output)
}
