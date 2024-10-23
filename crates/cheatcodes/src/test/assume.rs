use crate::{Cheatcode, Cheatcodes, CheatsCtxt, Error, Result};
use alloy_primitives::{Address, Bytes};
use foundry_common::ContractsByArtifact;
use foundry_evm_core::constants::MAGIC_ASSUME;
use revm::interpreter::InstructionResult;
use spec::Vm::{
    assumeCall, assumeNoPartialRevert_0Call, assumeNoPartialRevert_1Call, assumeNoRevert_0Call,
    assumeNoRevert_1Call, assumeNoRevert_2Call, assumeNoRevert_3Call, assumeNoRevert_4Call,
};
use std::fmt::Debug;

use super::revert::{handle_revert, RevertParameters};

#[derive(Clone, Debug)]
pub struct AssumeNoRevert {
    /// The call depth at which the cheatcode was added.
    pub depth: u64,
    /// Acceptable revert parameters for the next call, to be thrown out if they are encountered;
    /// reverts with parameters not specified here will count as normal reverts and not rejects
    /// towards the counter.
    pub reasons: Option<Vec<AcceptableRevertParameters>>,
}

/// Parameters for a single anticipated revert, to be thrown out if encountered.
#[derive(Clone, Debug)]
pub struct AcceptableRevertParameters {
    /// The expected revert data returned by the revert
    pub reason: Vec<u8>,
    /// If true then only the first 4 bytes of expected data returned by the revert are checked.
    pub partial_match: bool,
    /// Contract expected to revert next call.
    pub reverter: Option<Address>,
    /// Actual reverter of the call.
    pub reverted_by: Option<Address>,
}

impl RevertParameters for AcceptableRevertParameters {
    fn reverter(&self) -> Option<Address> {
        self.reverter
    }

    fn reason(&self) -> Option<&[u8]> {
        Some(&self.reason)
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
impl Cheatcode for assumeNoRevert_3Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { revertData, reverter } = self;
        assume_no_revert(
            ccx.state,
            ccx.ecx.journaled_state.depth(),
            Some(revertData.to_vec()),
            false,
            Some(*reverter),
        )
    }
}
impl Cheatcode for assumeNoRevert_4Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { revertData, reverter } = self;
        assume_no_revert(
            ccx.state,
            ccx.ecx.journaled_state.depth(),
            Some(revertData.to_vec()),
            false,
            Some(*reverter),
        )
    }
}

impl Cheatcode for assumeNoPartialRevert_0Call {
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

impl Cheatcode for assumeNoPartialRevert_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { revertData, reverter } = self;
        assume_no_revert(
            ccx.state,
            ccx.ecx.journaled_state.depth(),
            Some(revertData.to_vec()),
            true,
            Some(*reverter),
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
    ensure!(state.expected_revert.is_none(), "");

    // if assume_no_revert is not set, set it
    if state.assume_no_revert.is_none() {
        state.assume_no_revert = Some(AssumeNoRevert { depth, reasons: None });
        // if reason is not none, create a new AssumeNoRevertParams vec
        if let Some(reason) = reason {
            state.assume_no_revert.as_mut().unwrap().reasons =
                Some(vec![AcceptableRevertParameters {
                    reason,
                    partial_match,
                    reverter,
                    reverted_by: None,
                }]);
        }
    } else {
        // otherwise, ensure that reasons vec is not none and new reason is also not none
        let valid_assume =
            state.assume_no_revert.as_ref().unwrap().reasons.is_some() && reason.is_some();
        ensure!(
            valid_assume,
            "cannot combine a generic assumeNoRevert with specific assumeNoRevert reasons"
        );
        // and append the new reason
        state.assume_no_revert.as_mut().unwrap().reasons.as_mut().unwrap().push(
            AcceptableRevertParameters {
                reason: reason.unwrap(),
                partial_match,
                reverter,
                reverted_by: None,
            },
        );
    }

    Ok(Default::default())
}

pub(crate) fn handle_assume_no_revert(
    assume_no_revert: &AssumeNoRevert,
    status: InstructionResult,
    retdata: &Bytes,
    known_contracts: &Option<ContractsByArtifact>,
    reverter: Option<&Address>,
) -> Result<(), Error> {
    // iterate over acceptable reasons and try to match against any, otherwise, return an Error with
    // the revert data
    assume_no_revert
        .reasons
        .as_ref()
        .and_then(|reasons| {
            reasons.iter().find_map(|reason| {
                handle_revert(false, reason, status, retdata, known_contracts, reverter).ok()
            })
        })
        .ok_or_else(|| retdata.clone().into())
}
