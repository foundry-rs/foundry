//! Implementations of [`Scripting`](crate::Group::Scripting) cheatcodes.

use super::{Cheatcode, CheatsCtxt, DatabaseExt, Result};
use crate::Vm::*;
use alloy_primitives::{Address, U256};
use ethers::signers::Signer;
use foundry_config::Config;

impl Cheatcode for broadcast_0Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self {} = self;
        broadcast(ccx, None, true)
    }
}

impl Cheatcode for broadcast_1Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { signer } = self;
        broadcast(ccx, Some(signer), true)
    }
}

impl Cheatcode for broadcast_2Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { privateKey } = self;
        broadcast_key(ccx, privateKey, true)
    }
}

impl Cheatcode for startBroadcast_0Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self {} = self;
        broadcast(ccx, None, false)
    }
}

impl Cheatcode for startBroadcast_1Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { signer } = self;
        broadcast(ccx, Some(signer), false)
    }
}

impl Cheatcode for startBroadcast_2Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { privateKey } = self;
        broadcast_key(ccx, privateKey, false)
    }
}

impl Cheatcode for stopBroadcastCall {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self {} = self;
        ensure!(ccx.state.broadcast.is_some(), "no broadcast in progress to stop");
        ccx.state.broadcast = None;
        Ok(Default::default())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Broadcast {
    /// Address of the transaction origin
    pub new_origin: Address,
    /// Original caller
    pub original_caller: Address,
    /// Original `tx.origin`
    pub original_origin: Address,
    /// Depth of the broadcast
    pub depth: u64,
    /// Whether the prank stops by itself after the next call
    pub single_call: bool,
}

/// Sets up broadcasting from a script using `new_origin` as the sender.
fn broadcast<DB: DatabaseExt>(
    ccx: &mut CheatsCtxt<DB>,
    new_origin: Option<&Address>,
    single_call: bool,
) -> Result {
    ensure!(
        ccx.state.prank.is_none(),
        "you have an active prank; broadcasting and pranks are not compatible"
    );
    ensure!(ccx.state.broadcast.is_none(), "a broadcast is active already");

    correct_sender_nonce(ccx)?;

    ccx.state.broadcast = Some(Broadcast {
        new_origin: *new_origin.unwrap_or(&ccx.data.env.tx.caller),
        original_origin: ccx.caller,
        original_caller: ccx.data.env.tx.caller,
        depth: ccx.data.journaled_state.depth(),
        single_call,
    });
    Ok(Default::default())
}

/// Sets up broadcasting from a script with the sender derived from `private_key`.
/// Adds this private key to `state`'s `script_wallets` vector to later be used for signing
/// if broadcast is successful.
fn broadcast_key<DB: DatabaseExt>(
    ccx: &mut CheatsCtxt<DB>,
    private_key: &U256,
    single_call: bool,
) -> Result {
    let wallet = super::utils::parse_wallet(private_key)?.with_chain_id(ccx.data.env.cfg.chain_id);
    let new_origin = &wallet.address().0.into();

    let result = broadcast(ccx, Some(new_origin), single_call);
    if result.is_ok() {
        ccx.state.script_wallets.push(wallet);
    }
    result
}

/// When using `forge script`, the script method is called using the address from `--sender`.
/// That leads to its nonce being incremented by `call_raw`. In a `broadcast` scenario this is
/// undesirable. Therefore, we make sure to fix the sender's nonce **once**.
pub(super) fn correct_sender_nonce<DB: DatabaseExt>(ccx: &mut CheatsCtxt<DB>) -> Result<()> {
    let caller = ccx.data.env.tx.caller;
    if !ccx.state.corrected_nonce && caller.0 != Config::DEFAULT_SENDER.0 {
        let account = super::evm::journaled_account(ccx.data, caller)?;
        account.info.nonce = account.info.nonce.saturating_sub(1);
        ccx.state.corrected_nonce = true;
    }
    Ok(())
}
