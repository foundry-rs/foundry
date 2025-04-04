//! Implementations of [`Testing`](spec::Group::Testing) cheatcodes.

use crate::{Cheatcode, Cheatcodes, CheatsCtxt, Result, Vm::*};
use alloy_primitives::{Address, U256};
use alloy_sol_types::SolValue;
use foundry_common::version::SEMVER_VERSION;
use foundry_evm_core::constants::MAGIC_SKIP;

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

impl Cheatcode for getChainCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { chainAlias } = self;
        let chain = get_chain_data(state, chainAlias)?;
        Ok(chain.abi_encode())
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
            ensure!(ccx.ecx.journaled_state.depth() <= 1, "`skip` can only be used at test level");
            Err([MAGIC_SKIP, reason.as_bytes()].concat().into())
        } else {
            Ok(Default::default())
        }
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

/// Get chain data and create a Chain struct
fn get_chain_data(state: &Cheatcodes, alias: &str) -> Result<Chain> {
    // Ensure the alias is not empty
    if alias.is_empty() {
        bail!("Chain alias cannot be empty");
    }
    // Get chain data from alias
    let chain_data = state.config.get_chain_data_by_alias_non_mut(alias)?;
    // Get RPC URL
    let rpc_url = state.config.get_rpc_url_non_mut(alias)?;
    // Create the Chain struct
    Ok(Chain {
        name: chain_data.name,
        chainId: U256::from(chain_data.chain_id),
        chainAlias: alias.to_string(),
        rpcUrl: rpc_url,
    })
}
