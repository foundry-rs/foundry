//! Implementations of [`Testing`](crate::Group::Testing) cheatcodes.

use crate::{Cheatcode, Cheatcodes, CheatsCtxt, DatabaseExt, Error, Result, Vm::*};
use alloy_primitives::Address;
use alloy_sol_types::SolValue;
use foundry_evm_core::constants::{MAGIC_ASSUME, MAGIC_SKIP};

pub(crate) mod expect;

impl Cheatcode for assumeCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { condition } = self;
        if *condition {
            Ok(Default::default())
        } else {
            Err(Error::from(MAGIC_ASSUME))
        }
    }
}

impl Cheatcode for breakpoint_0Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { char } = self;
        breakpoint(ccx.state, &ccx.caller, char, true)
    }
}

impl Cheatcode for breakpoint_1Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { char, value } = self;
        breakpoint(ccx.state, &ccx.caller, char, *value)
    }
}

impl Cheatcode for rpcUrlCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { rpcAlias } = self;
        state.config.rpc_url(rpcAlias).map(|url| url.abi_encode())
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

impl Cheatcode for skipCall {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { skipTest } = *self;
        if skipTest {
            // Skip should not work if called deeper than at test level.
            // Since we're not returning the magic skip bytes, this will cause a test failure.
            ensure!(ccx.data.journaled_state.depth() <= 1, "`skip` can only be used at test level");
            Err(MAGIC_SKIP.into())
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
