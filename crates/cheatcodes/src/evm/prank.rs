use crate::{Cheatcode, CheatsCtxt, Result, Vm::*, evm::journaled_account};
use alloy_primitives::Address;
use foundry_evm_core::evm::FoundryEvmNetwork;

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
    pub depth: usize,
    /// Whether the prank stops by itself after the next call
    pub single_call: bool,
    /// Whether the prank should be applied to delegate call
    pub delegate_call: bool,
    /// Whether the prank has been used yet (false if unused)
    pub used: bool,
}

impl Prank {
    /// Create a new prank.
    pub const fn new(
        prank_caller: Address,
        prank_origin: Address,
        new_caller: Address,
        new_origin: Option<Address>,
        depth: usize,
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
    pub const fn first_time_applied(&self) -> Option<Self> {
        if self.used { None } else { Some(Self { used: true, ..*self }) }
    }

    /// Returns how the prank changes a call or create from `caller` at `depth`, if it applies.
    pub(crate) fn changes_for(&self, depth: usize, caller: Address) -> Option<PrankChanges> {
        if depth < self.depth || caller != self.prank_caller {
            return None;
        }

        // At the target depth we set `msg.sender`.
        let new_caller = (depth == self.depth).then_some(self.new_caller);

        // At the target depth, or deeper, we set `tx.origin`.
        let applied = new_caller.is_some() || self.new_origin.is_some();

        Some(PrankChanges {
            caller: new_caller,
            origin: self.new_origin,
            used: if applied { self.first_time_applied() } else { None },
        })
    }
}

/// How a prank changes a call or create.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PrankChanges {
    /// The new `msg.sender`, set only at the prank depth.
    pub(crate) caller: Option<Address>,
    /// The new `tx.origin`.
    pub(crate) origin: Option<Address>,
    /// The prank marked as used, if this is its first application.
    pub(crate) used: Option<Prank>,
}

impl Cheatcode for prank_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { msgSender } = self;
        prank(ccx, msgSender, None, true, false)
    }
}

impl Cheatcode for startPrank_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { msgSender } = self;
        prank(ccx, msgSender, None, false, false)
    }
}

impl Cheatcode for prank_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { msgSender, txOrigin } = self;
        prank(ccx, msgSender, Some(txOrigin), true, false)
    }
}

impl Cheatcode for startPrank_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { msgSender, txOrigin } = self;
        prank(ccx, msgSender, Some(txOrigin), false, false)
    }
}

impl Cheatcode for prank_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { msgSender, delegateCall } = self;
        prank(ccx, msgSender, None, true, *delegateCall)
    }
}

impl Cheatcode for startPrank_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { msgSender, delegateCall } = self;
        prank(ccx, msgSender, None, false, *delegateCall)
    }
}

impl Cheatcode for prank_3Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { msgSender, txOrigin, delegateCall } = self;
        prank(ccx, msgSender, Some(txOrigin), true, *delegateCall)
    }
}

impl Cheatcode for startPrank_3Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { msgSender, txOrigin, delegateCall } = self;
        prank(ccx, msgSender, Some(txOrigin), false, *delegateCall)
    }
}

impl Cheatcode for stopPrankCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        ccx.state.pranks.remove(&ccx.depth());
        Ok(Default::default())
    }
}

fn prank<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    new_caller: &Address,
    new_origin: Option<&Address>,
    single_call: bool,
    delegate_call: bool,
) -> Result {
    // Ensure that we load the account of the pranked address and mark it as touched.
    // This is necessary to ensure that account state changes (such as the account's `nonce`) are
    // properly tracked.
    let account = journaled_account(ccx.ecx, *new_caller)?;

    // Ensure that code exists at `msg.sender` if delegate calling.
    if delegate_call {
        ensure!(
            account.info.code.as_ref().is_some_and(|code| !code.is_empty()),
            "cannot `prank` delegate call from an EOA"
        );
    }

    let depth = ccx.depth();
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
        ccx.tx_caller(),
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

#[cfg(test)]
mod tests {
    use super::*;

    const CALLER: Address = Address::repeat_byte(0x01);
    const SENDER: Address = Address::repeat_byte(0x02);
    const ORIGIN: Address = Address::repeat_byte(0x03);

    #[test]
    fn changes_for_applies_the_sender_only_at_the_prank_depth() {
        let prank = Prank::new(CALLER, Address::ZERO, SENDER, None, 2, false, false);
        assert!(prank.changes_for(1, CALLER).is_none());
        assert!(prank.changes_for(2, ORIGIN).is_none());

        let at_depth = prank.changes_for(2, CALLER).unwrap();
        assert_eq!((at_depth.caller, at_depth.origin), (Some(SENDER), None));
        let used = at_depth.used.unwrap();
        assert!(used.used);
        assert!(used.changes_for(2, CALLER).unwrap().used.is_none());

        let deeper = prank.changes_for(3, CALLER).unwrap();
        assert_eq!((deeper.caller, deeper.origin), (None, None));
        assert!(deeper.used.is_none());

        let with_origin = Prank::new(CALLER, Address::ZERO, SENDER, Some(ORIGIN), 2, false, false);
        let deeper = with_origin.changes_for(3, CALLER).unwrap();
        assert_eq!((deeper.caller, deeper.origin), (None, Some(ORIGIN)));
        assert!(deeper.used.is_some());
    }
}
