//! Execution-engine-independent prank state.

use alloy_primitives::Address;

/// A prank that changes the caller of a later call or creation.
#[derive(Clone, Copy, Debug, Default)]
pub struct Prank {
    /// Address of the contract that initiated the prank.
    pub prank_caller: Address,
    /// Transaction origin when the prank was initiated.
    pub prank_origin: Address,
    /// Address assigned to `msg.sender`.
    pub new_caller: Address,
    /// Address assigned to `tx.origin`, if requested.
    pub new_origin: Option<Address>,
    /// Call depth at which the prank was created.
    pub depth: usize,
    /// Whether the prank ends after one call or creation.
    pub single_call: bool,
    /// Whether the prank targets a delegate call.
    pub delegate_call: bool,
    /// Whether the prank has been applied at least once.
    pub used: bool,
}

impl Prank {
    /// Creates a prank at the current call depth.
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

    /// Returns a copy marked as applied on first use.
    pub const fn first_time_applied(&self) -> Option<Self> {
        if self.used { None } else { Some(Self { used: true, ..*self }) }
    }
}
