//! Context-free testing cheatcodes shared by both engines.

use crate::{Cheatcode, Result, Vm::*};
use alloy_sol_types::SolValue;
use foundry_common::version::SEMVER_VERSION;

impl Cheatcode for getFoundryVersionCall {
    fn apply_context_free(&self) -> Result {
        let Self {} = self;
        Ok(SEMVER_VERSION.abi_encode())
    }
}

impl Cheatcode for sleepCall {
    fn apply_context_free(&self) -> Result {
        let Self { duration } = self;
        let sleep_duration = std::time::Duration::from_millis(duration.saturating_to());
        std::thread::sleep(sleep_duration);
        Ok(Default::default())
    }
}
