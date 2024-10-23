use crate::{Cheatcode, Cheatcodes, CheatsCtxt, Error, Result};
use alloy_primitives::{Address, Bytes};
use foundry_common::ContractsByArtifact;
use foundry_evm_core::constants::MAGIC_ASSUME;
use revm::interpreter::InstructionResult;
use spec::Vm::{
    assumeCall, assumeNoPartialRevertCall, assumeNoRevert_0Call, assumeNoRevert_1Call,
    assumeNoRevert_2Call,
};
use std::fmt::Debug;

use super::revert::{handle_revert, RevertParameters};

#[derive(Clone, Debug)]
pub struct AssumeNoRevert {
    /// The call depth at which the cheatcode was added.
    pub depth: u64,
    /// The expected revert data returned by the revert, None being any.
    pub reason: Option<Vec<u8>>,
    /// If true then only the first 4 bytes of expected data returned by the revert are checked.
    pub partial_match: bool,
    /// Contract expected to revert next call.
    pub reverter: Option<Address>,
    /// Actual reverter of the call.
    pub reverted_by: Option<Address>,
}

impl RevertParameters for AssumeNoRevert {
    fn reverter(&self) -> Option<Address> {
        self.reverter
    }

    fn reverted_by(&self) -> Option<Address> {
        self.reverted_by
    }

    fn reason(&self) -> Option<&[u8]> {
        self.reason.as_deref()
    }

    fn partial_match(&self) -> bool {
        self.partial_match
    }
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

impl Cheatcode for assumeNoRevert_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        assume_no_revert(ccx.state, ccx.ecx.journaled_state.depth(), None, false, None)
    }
}

impl Cheatcode for assumeNoRevert_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { revertData } = self;
        assume_no_revert(
            ccx.state,
            ccx.ecx.journaled_state.depth(),
            Some(revertData.to_vec()),
            false,
            None,
        )
    }
}
impl Cheatcode for assumeNoRevert_2Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { revertData } = self;
        assume_no_revert(
            ccx.state,
            ccx.ecx.journaled_state.depth(),
            Some(revertData.to_vec()),
            false,
            None,
        )
    }
}

impl Cheatcode for assumeNoPartialRevertCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { revertData } = self;
        assume_no_revert(
            ccx.state,
            ccx.ecx.journaled_state.depth(),
            Some(revertData.to_vec()),
            true,
            None,
        )
    }
}

fn assume_no_revert(
    state: &mut Cheatcodes,
    depth: u64,
    reason: Option<Vec<u8>>,
    partial_match: bool,
    reverter: Option<Address>,
) -> Result {
    // todo: support multiple assumeNoRevert calls; use a Vec<Vec<u8>> for reason to support
    // multiple reasons
    // check exists, push to vec
    // else create new one-element vec if reason is Some
    // will also need to ensure cheatcodes don't reset this
    state.assume_no_revert =
        Some(AssumeNoRevert { depth, reason, partial_match, reverter, reverted_by: None });
    Ok(Default::default())
}

pub(crate) fn handle_assume_no_revert(
    assume_no_revert: &AssumeNoRevert,
    status: InstructionResult,
    retdata: Bytes,
    known_contracts: &Option<ContractsByArtifact>,
) -> Result<(), Error> {
    handle_revert(false, assume_no_revert, status, retdata, known_contracts)
}
