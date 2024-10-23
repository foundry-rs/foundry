use crate::{Cheatcode, Cheatcodes, CheatsCtxt, Error, Result};
use alloy_primitives::Address;
use foundry_evm_core::constants::MAGIC_ASSUME;
use spec::Vm::{
    assumeCall, assumeNoRevert_0Call, assumeNoRevert_1Call, assumeNoRevert_2Call,
    assumeNoRevert_3Call, assumeNoRevert_4Call,
};
use std::fmt::Debug;

pub const ASSUME_EXPECT_REJECT_MAGIC: &str = "Cannot combine an assumeNoRevert with expectRevert";
pub const ASSUME_REJECT_MAGIC: &str =
    "Cannot combine a generic assumeNoRevert with specific assumeNoRevert reasons";

#[derive(Clone, Debug)]
pub struct AssumeNoRevert {
    /// The call depth at which the cheatcode was added.
    pub depth: u64,
    /// Acceptable revert parameters for the next call, to be thrown out if they are encountered;
    /// reverts with parameters not specified here will count as normal reverts and not rejects
    /// towards the counter.
    pub reasons: Vec<AcceptableRevertParameters>,
    /// Address that reverted the call.
    pub reverted_by: Option<Address>,
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
        assume_no_revert(ccx.state, ccx.ecx.journaled_state.depth(), None, None)
    }
}

impl Cheatcode for assumeNoRevert_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { revertData } = self;
        assume_no_revert(
            ccx.state,
            ccx.ecx.journaled_state.depth(),
            Some(revertData.to_vec()),
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
            Some(*reverter),
        )
    }
}

fn assume_no_revert(
    state: &mut Cheatcodes,
    depth: u64,
    reason: Option<Vec<u8>>,
    reverter: Option<Address>,
) -> Result {
    ensure!(state.expected_revert.is_none(), ASSUME_EXPECT_REJECT_MAGIC);

    let params = reason.map(|reason| {
        let partial_match = reason.len() == 4;
        AcceptableRevertParameters { reason, partial_match, reverter }
    });

    match state.assume_no_revert {
        Some(ref mut assume) => {
            ensure!(!assume.reasons.is_empty() && params.is_some(), ASSUME_REJECT_MAGIC);
            assume.reasons.push(params.unwrap());
        }
        None => {
            state.assume_no_revert = Some(AssumeNoRevert {
                depth,
                reasons: if let Some(params) = params { vec![params] } else { vec![] },
                reverted_by: None,
            });
        }
    }

    Ok(Default::default())
}
