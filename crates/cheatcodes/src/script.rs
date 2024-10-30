//! Implementations of [`Scripting`](spec::Group::Scripting) cheatcodes.

use crate::{Cheatcode, CheatsCtxt, Result, Vm::*};
use alloy_primitives::{Address, B256, U256};
use alloy_rpc_types::Authorization;
use alloy_signer::{Signature, SignerSync};
use alloy_sol_types::SolValue;
use alloy_signer_local::PrivateKeySigner;
use foundry_wallets::{multi_wallet::MultiWallet, WalletSigner};
use parking_lot::Mutex;
use revm::primitives::SignedAuthorization;
use std::sync::Arc;

impl Cheatcode for broadcast_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self {} = self;
        broadcast(ccx, None, true)
    }
}

impl Cheatcode for broadcast_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { signer } = self;
        broadcast(ccx, Some(signer), true)
    }
}

impl Cheatcode for broadcast_2Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { privateKey } = self;
        broadcast_key(ccx, privateKey, true)
    }
}

impl Cheatcode for createDelegationCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { implementation, nonce } = self;
        let auth = Authorization {
            address: *implementation,
            nonce: *nonce,
            chain_id: U256::from(ccx.ecx.env.cfg.chain_id),
        };
        let hash = auth.signature_hash();
        Ok(hash.to_vec())
    }
}

impl Cheatcode for attachDelegationCall {
    // @todo - change this to apply_full?
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { implementation, nonce, v, r, s } = self;
        let auth = Authorization {
            address: *implementation,
            nonce: *nonce,
            chain_id: U256::from(ccx.ecx.env.cfg.chain_id),
        };
        let addr = auth.address;
        let sig = Signature::from_rs_and_parity(
            U256::try_from(*r).unwrap(),
            U256::try_from(*s).unwrap(),
            alloy_primitives::Parity::from(*v == 1)
        )?;
        let signed_auth = auth.into_signed(sig);
        // store in CheatcodeState
        ccx.state.delegations.insert(addr, signed_auth);
        Ok(Default::default())
    }
}

impl Cheatcode for signDelegationCall {
    fn apply_stateful(&self, _: &mut CheatsCtxt) -> Result {
        let Self { delegation, privateKey } = self;
        let signer = super::crypto::parse_wallet(privateKey)?;
        let sig = signer.sign_hash_sync(delegation)?;
        Ok(encode_delegation_sig(sig))
    }
}

fn encode_delegation_sig(sig: alloy_primitives::Signature) -> Vec<u8> {
    let v = U256::from(sig.v().y_parity() as u8);
    let r = sig.r();
    let s = sig.s();
    (v, r, s).abi_encode()
}

impl Cheatcode for startBroadcast_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self {} = self;
        broadcast(ccx, None, false)
    }
}

impl Cheatcode for startBroadcast_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { signer } = self;
        broadcast(ccx, Some(signer), false)
    }
}

impl Cheatcode for startBroadcast_2Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { privateKey } = self;
        broadcast_key(ccx, privateKey, false)
    }
}

impl Cheatcode for stopBroadcastCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
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
    /// EIP-7702 delegation
    pub delegation: Option<SignedAuthorization>,
}

/// Contains context for wallet management.
#[derive(Debug)]
pub struct ScriptWalletsInner {
    /// All signers in scope of the script.
    pub multi_wallet: MultiWallet,
    /// Optional signer provided as `--sender` flag.
    pub provided_sender: Option<Address>,
}

/// Clonable wrapper around [`ScriptWalletsInner`].
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
    pub fn add_private_key(&self, private_key: &B256) -> Result<()> {
        self.add_local_signer(PrivateKeySigner::from_bytes(private_key)?);
        Ok(())
    }

    /// Locks inner Mutex and adds a signer to the [MultiWallet].
    pub fn add_local_signer(&self, wallet: PrivateKeySigner) {
        self.inner.lock().multi_wallet.add_signer(WalletSigner::Local(wallet));
    }

    /// Locks inner Mutex and returns all signer addresses in the [MultiWallet].
    pub fn signers(&self) -> Result<Vec<Address>> {
        Ok(self.inner.lock().multi_wallet.signers()?.keys().cloned().collect())
    }
}

/// Sets up broadcasting from a script using `new_origin` as the sender.
fn broadcast(ccx: &mut CheatsCtxt, new_origin: Option<&Address>, single_call: bool) -> Result {
    ensure!(
        ccx.state.prank.is_none(),
        "you have an active prank; broadcasting and pranks are not compatible"
    );
    ensure!(ccx.state.broadcast.is_none(), "a broadcast is active already");

    let mut new_origin = new_origin.cloned();
    let mut delegation = None;

    if new_origin.is_none() {
        if let Some(script_wallets) = ccx.state.script_wallets() {
            let mut script_wallets = script_wallets.inner.lock();
            if let Some(provided_sender) = script_wallets.provided_sender {
                new_origin = Some(provided_sender);
                // check if this sender has a delegation
                delegation = ccx.state.delegations.get(&provided_sender).cloned();
            } else {
                let signers = script_wallets.multi_wallet.signers()?;
                if signers.len() == 1 {
                    let address = signers.keys().next().unwrap();
                    new_origin = Some(*address);
                    // check delegation for single signer case
                    delegation = ccx.state.delegations.get(address).cloned();
                }
            }
        }
    } else if let Some(addr) = new_origin {
        // check delegation for explicitly provided address
        delegation = ccx.state.delegations.get(&addr).cloned();
    }

    let broadcast = Broadcast {
        new_origin: new_origin.unwrap_or(ccx.ecx.env.tx.caller),
        original_caller: ccx.caller,
        original_origin: ccx.ecx.env.tx.caller,
        depth: ccx.ecx.journaled_state.depth(),
        single_call,
        delegation
    };
    debug!(target: "cheatcodes", ?broadcast, "started");
    ccx.state.broadcast = Some(broadcast);
    Ok(Default::default())
}

/// Sets up broadcasting from a script with the sender derived from `private_key`.
/// Adds this private key to `state`'s `script_wallets` vector to later be used for signing
/// if broadcast is successful.
fn broadcast_key(ccx: &mut CheatsCtxt, private_key: &U256, single_call: bool) -> Result {
    let wallet = super::crypto::parse_wallet(private_key)?;
    let new_origin = wallet.address();

    let result = broadcast(ccx, Some(&new_origin), single_call);
    if result.is_ok() {
        if let Some(script_wallets) = ccx.state.script_wallets() {
            script_wallets.add_local_signer(wallet);
        }
    }
    result
}
