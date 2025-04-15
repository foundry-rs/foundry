use crate::{Cheatcode, CheatsCtxt, Result, Vm::*};
use alloy_primitives::Address;

/// Prank information.
#[derive(Clone, Copy, Debug, Default)]
pub struct Prank {
    /// Address of the contract that initiated the prank
    pub prank_caller: Address,
    /// Address of `tx.origin` when the prank was initiated
    pub prank_origin: Address,
    /// The address to assign to `msg.sender`
    pub new_caller: Address,
    /// The address to assign to `tx.origin`
    pub new_origin: Option<Address>,
    /// The depth at which the prank was called
    pub depth: u64,
    /// Whether the prank stops by itself after the next call
    pub single_call: bool,
    /// Whether the prank should be be applied to delegate call
    pub delegate_call: bool,
    /// Whether the prank has been used yet (false if unused)
    pub used: bool,
}

impl Prank {
    /// Create a new prank.
    pub fn new(
        prank_caller: Address,
        prank_origin: Address,
        new_caller: Address,
        new_origin: Option<Address>,
        depth: u64,
        single_call: bool,
        delegate_call: bool,
    ) -> Self {
        Self {
            prank_caller,
            prank_origin,
            new_caller,
            new_origin,
            depth,
            single_call,
            delegate_call,
            used: false,
        }
    }

    /// Apply the prank by setting `used` to true if it is false
    /// Only returns self in the case it is updated (first application)
    pub fn first_time_applied(&self) -> Option<Self> {
        if self.used {
            None
        } else {
            Some(Self { used: true, ..*self })
        }
    }
}

impl Cheatcode for prank_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { msgSender } = self;
        prank(ccx, msgSender, None, true, false)
    }
}

impl Cheatcode for startPrank_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { msgSender } = self;
        prank(ccx, msgSender, None, false, false)
    }
}

impl Cheatcode for prank_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { msgSender, txOrigin } = self;
        prank(ccx, msgSender, Some(txOrigin), true, false)
    }
}

impl Cheatcode for startPrank_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { msgSender, txOrigin } = self;
        prank(ccx, msgSender, Some(txOrigin), false, false)
    }
}

impl Cheatcode for prank_2Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { msgSender, delegateCall } = self;
        prank(ccx, msgSender, None, true, *delegateCall)
    }
}

impl Cheatcode for startPrank_2Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { msgSender, delegateCall } = self;
        prank(ccx, msgSender, None, false, *delegateCall)
    }
}

impl Cheatcode for prank_3Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { msgSender, txOrigin, delegateCall } = self;
        prank(ccx, msgSender, Some(txOrigin), true, *delegateCall)
    }
}

impl Cheatcode for startPrank_3Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { msgSender, txOrigin, delegateCall } = self;
        prank(ccx, msgSender, Some(txOrigin), false, *delegateCall)
    }
}

impl Cheatcode for stopPrankCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self {} = self;
        ccx.state.pranks.remove(&ccx.ecx.journaled_state.depth());
        Ok(Default::default())
    }
}

fn prank(
    ccx: &mut CheatsCtxt,
    new_caller: &Address,
    new_origin: Option<&Address>,
    single_call: bool,
    delegate_call: bool,
) -> Result {
    // Ensure that code exists at `msg.sender` if delegate calling.
    if delegate_call {
        let code = ccx.code(*new_caller)?;
        ensure!(!code.is_empty(), "cannot `prank` delegate call from an EOA");
    }

    let depth = ccx.ecx.journaled_state.depth();
    if let Some(Prank { used, single_call: current_single_call, .. }) = ccx.state.get_prank(depth) {
        ensure!(used, "cannot overwrite a prank until it is applied at least once");
        // This case can only fail if the user calls `vm.startPrank` and then `vm.prank` later on.
        // This should not be possible without first calling `stopPrank`
        ensure!(
            single_call == *current_single_call,
            "cannot override an ongoing prank with a single vm.prank; \
             use vm.startPrank to override the current prank"
        );
    }

    let prank = Prank::new(
        ccx.caller,
        ccx.ecx.env.tx.caller,
        *new_caller,
        new_origin.copied(),
        depth,
        single_call,
        delegate_call,
    );

    ensure!(
        ccx.state.broadcast.is_none(),
        "cannot `prank` for a broadcasted transaction; \
         pass the desired `tx.origin` into the `broadcast` cheatcode call"
    );

    ccx.state.pranks.insert(prank.depth, prank);
    Ok(Default::default())
}
