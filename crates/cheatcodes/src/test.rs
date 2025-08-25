//! Implementations of [`Testing`](spec::Group::Testing) cheatcodes.

use crate::{Cheatcode, Cheatcodes, CheatsCtxt, Result, Vm::*};
use alloy_chains::Chain as AlloyChain;
use alloy_primitives::{Address, U256};
use alloy_sol_types::SolValue;
use foundry_common::version::SEMVER_VERSION;
use foundry_evm_core::constants::MAGIC_SKIP;
use std::str::FromStr;

pub(crate) mod assert;
pub(crate) mod assume;
pub(crate) mod expect;
pub(crate) mod revert_handlers;

impl Cheatcode for breakpoint_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { char } = self;
        breakpoint(ccx.state, &ccx.caller, char, true)
    }
}

impl Cheatcode for breakpoint_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { char, value } = self;
        breakpoint(ccx.state, &ccx.caller, char, *value)
    }
}

impl Cheatcode for getFoundryVersionCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        Ok(SEMVER_VERSION.abi_encode())
    }
}

impl Cheatcode for rpcUrlCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { rpcAlias } = self;
        let url = state.config.rpc_endpoint(rpcAlias)?.url()?.abi_encode();
        Ok(url)
    }
}

impl Cheatcode for rpcUrlsCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        state.config.rpc_urls().map(|urls| urls.abi_encode())
    }
}

impl Cheatcode for rpcUrlStructsCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        state.config.rpc_urls().map(|urls| urls.abi_encode())
    }
}

impl Cheatcode for sleepCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { duration } = self;
        let sleep_duration = std::time::Duration::from_millis(duration.saturating_to());
        std::thread::sleep(sleep_duration);
        Ok(Default::default())
    }
}

impl Cheatcode for skip_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { skipTest } = *self;
        skip_1Call { skipTest, reason: String::new() }.apply_stateful(ccx)
    }
}

impl Cheatcode for skip_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { skipTest, reason } = self;
        if *skipTest {
            // Skip should not work if called deeper than at test level.
            // Since we're not returning the magic skip bytes, this will cause a test failure.
            ensure!(ccx.ecx.journaled_state.depth <= 1, "`skip` can only be used at test level");
            Err([MAGIC_SKIP, reason.as_bytes()].concat().into())
        } else {
            Ok(Default::default())
        }
    }
}

impl Cheatcode for getChain_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { chainAlias } = self;
        get_chain(state, chainAlias)
    }
}

impl Cheatcode for getChain_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { chainId } = self;
        // Convert the chainId to a string and use the existing get_chain function
        let chain_id_str = chainId.to_string();
        get_chain(state, &chain_id_str)
    }
}

/// Adds or removes the given breakpoint to the state.
fn breakpoint(state: &mut Cheatcodes, caller: &Address, s: &str, add: bool) -> Result {
    let mut chars = s.chars();
    let (Some(point), None) = (chars.next(), chars.next()) else {
        bail!("breakpoints must be exactly one character");
    };
    ensure!(point.is_alphabetic(), "only alphabetic characters are accepted as breakpoints");

    if add {
        state.breakpoints.insert(point, (*caller, state.pc));
    } else {
        state.breakpoints.remove(&point);
    }

    Ok(Default::default())
}

/// Gets chain information for the given alias.
fn get_chain(state: &mut Cheatcodes, chain_alias: &str) -> Result {
    // Parse the chain alias - works for both chain names and IDs
    let alloy_chain = AlloyChain::from_str(chain_alias)
        .map_err(|_| fmt_err!("invalid chain alias: {chain_alias}"))?;
    let chain_name = alloy_chain.to_string();
    let chain_id = alloy_chain.id();

    // Check if this is an unknown chain ID by comparing the name to the chain ID
    // When a numeric ID is passed for an unknown chain, alloy_chain.to_string() will return the ID
    // So if they match, it's likely an unknown chain ID
    if chain_name == chain_id.to_string() {
        return Err(fmt_err!("invalid chain alias: {chain_alias}"));
    }

    // Try to retrieve RPC URL and chain alias from user's config in foundry.toml.
    let (rpc_url, chain_alias) = if let Some(rpc_url) =
        state.config.rpc_endpoint(&chain_name).ok().and_then(|e| e.url().ok())
    {
        (rpc_url, chain_name.clone())
    } else {
        (String::new(), chain_alias.to_string())
    };

    let chain_struct = Chain {
        name: chain_name,
        chainId: U256::from(chain_id),
        chainAlias: chain_alias,
        rpcUrl: rpc_url,
    };

    Ok(chain_struct.abi_encode())
}
