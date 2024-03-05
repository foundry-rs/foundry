//! Implementations of [`Scripting`](crate::Group::Scripting) cheatcodes.

use crate::{Cheatcode, CheatsCtxt, DatabaseExt, Result, Vm::*};
use alloy_primitives::{Address, U256};
use alloy_signer::{LocalWallet, Signer};
use foundry_config::Config;
use foundry_wallets::{multi_wallet::MultiWallet, WalletSigner};
use parking_lot::Mutex;
use std::sync::Arc;

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
        let Some(broadcast) = ccx.state.broadcast.take() else {
            bail!("no broadcast in progress to stop");
        };
        debug!(target: "cheatcodes", ?broadcast, "stopped");
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

/// Contains context for wallet management.
#[derive(Debug)]
pub struct ScriptWalletsInner {
    /// All signers in scope of the script.
    pub multi_wallet: MultiWallet,
    /// Optional signer provided as `--sender` flag.
    pub provided_sender: Option<Address>,
}

/// Clonable wrapper around [ScriptWalletsInner].
#[derive(Debug, Clone)]
pub struct ScriptWallets {
    /// Inner data.
    pub inner: Arc<Mutex<ScriptWalletsInner>>,
}

impl ScriptWallets {
    #[allow(missing_docs)]
    pub fn new(multi_wallet: MultiWallet, provided_sender: Option<Address>) -> Self {
        Self { inner: Arc::new(Mutex::new(ScriptWalletsInner { multi_wallet, provided_sender })) }
    }

    /// Consumes [ScriptWallets] and returns [MultiWallet].
    ///
    /// Panics if [ScriptWallets] is still in use.
    pub fn into_multi_wallet(self) -> MultiWallet {
        Arc::into_inner(self.inner)
            .map(|m| m.into_inner().multi_wallet)
            .unwrap_or_else(|| panic!("not all instances were dropped"))
    }

    /// Locks inner Mutex and adds a signer to the [MultiWallet].
    pub fn add_signer(&self, private_key: impl AsRef<[u8]>) -> Result {
        self.inner.lock().multi_wallet.add_signer(WalletSigner::from_private_key(private_key)?);
        Ok(Default::default())
    }
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

    let mut new_origin = new_origin.cloned();

    if new_origin.is_none() {
        if let Some(script_wallets) = &ccx.state.script_wallets {
            let mut script_wallets = script_wallets.inner.lock();
            if let Some(provided_sender) = script_wallets.provided_sender {
                new_origin = Some(provided_sender);
            } else {
                let signers = script_wallets.multi_wallet.signers()?;
                if signers.len() == 1 {
                    let address = signers.keys().next().unwrap();
                    new_origin = Some(*address);
                }
            }
        }
    }

    let broadcast = Broadcast {
        new_origin: new_origin.unwrap_or(ccx.data.env.tx.caller),
        original_caller: ccx.caller,
        original_origin: ccx.data.env.tx.caller,
        depth: ccx.data.journaled_state.depth(),
        single_call,
    };
    debug!(target: "cheatcodes", ?broadcast, "started");
    ccx.state.broadcast = Some(broadcast);
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
    let key = super::utils::parse_private_key(private_key)?;
    let new_origin = LocalWallet::from(key.clone()).address();

    let result = broadcast(ccx, Some(&new_origin), single_call);

    if result.is_ok() {
        if let Some(script_wallets) = &ccx.state.script_wallets {
            script_wallets.add_signer(key.to_bytes())?;
        }
    }
    result
}

/// When using `forge script`, the script method is called using the address from `--sender`.
/// That leads to its nonce being incremented by `call_raw`. In a `broadcast` scenario this is
/// undesirable. Therefore, we make sure to fix the sender's nonce **once**.
pub(super) fn correct_sender_nonce<DB: DatabaseExt>(ccx: &mut CheatsCtxt<DB>) -> Result<()> {
    let sender = ccx.data.env.tx.caller;
    if !ccx.state.corrected_nonce && sender != Config::DEFAULT_SENDER {
        let account = super::evm::journaled_account(ccx.data, sender)?;
        let prev = account.info.nonce;
        account.info.nonce = prev.saturating_sub(1);
        debug!(target: "cheatcodes", %sender, nonce=account.info.nonce, prev, "corrected nonce");
        ccx.state.corrected_nonce = true;
    }
    Ok(())
}
