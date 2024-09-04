use crate::{Cheatcode, Cheatcodes, CheatsCtxt, Error, Result};
use foundry_evm_core::{backend::DatabaseExt, constants::MAGIC_ASSUME};
use spec::Vm::{assumeCall, assumeNoRevertCall};
use std::fmt::Debug;

#[derive(Clone, Debug)]
pub struct AssumeNoRevert {
    /// The call depth at which the cheatcode was added.
    pub depth: u64,
}

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

impl Cheatcode for assumeNoRevertCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        ccx.state.assume_no_revert =
            Some(AssumeNoRevert { depth: ccx.ecx.journaled_state.depth() });
        Ok(Default::default())
    }
}
