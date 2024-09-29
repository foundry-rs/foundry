//! Implementations of [`Testing`](spec::Group::Testing) cheatcodes.

use chrono::DateTime;
use std::env;

use crate::{Cheatcode, Cheatcodes, CheatsCtxt, DatabaseExt, Result, Vm::*};
use alloy_primitives::Address;
use alloy_sol_types::SolValue;
use foundry_evm_core::constants::MAGIC_SKIP;

pub(crate) mod assert;
pub(crate) mod assume;
pub(crate) mod expect;

impl Cheatcode for breakpoint_0Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { char } = self;
        breakpoint(ccx.state, &ccx.caller, char, true)
    }
}

impl Cheatcode for breakpoint_1Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { char, value } = self;
        breakpoint(ccx.state, &ccx.caller, char, *value)
    }
}

impl Cheatcode for getFoundryVersionCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        let cargo_version = env!("CARGO_PKG_VERSION");
        let git_sha = env!("VERGEN_GIT_SHA");
        let build_timestamp = DateTime::parse_from_rfc3339(env!("VERGEN_BUILD_TIMESTAMP"))
            .expect("Invalid build timestamp format")
            .format("%Y%m%d%H%M")
            .to_string();
        let foundry_version = format!("{cargo_version}+{git_sha}+{build_timestamp}");
        Ok(foundry_version.abi_encode())
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

impl Cheatcode for skip_0Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { skipTest } = *self;
        skip_1Call { skipTest, reason: String::new() }.apply_stateful(ccx)
    }
}

impl Cheatcode for skip_1Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
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
