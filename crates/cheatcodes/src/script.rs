//! Implementations of [`Scripting`](spec::Group::Scripting) cheatcodes.

use crate::{Cheatcode, CheatsCtxt, Result, Vm::*};
use alloy_primitives::{Address, PrimitiveSignature, B256, U256};
use alloy_rpc_types::Authorization;
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::SolValue;
use foundry_wallets::{multi_wallet::MultiWallet, WalletSigner};
use parking_lot::Mutex;
use revm::primitives::{Bytecode, SignedAuthorization};
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

impl Cheatcode for attachDelegationCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { signedDelegation } = self;
        let SignedDelegation { v, r, s, nonce, implementation } = signedDelegation;

        let auth = Authorization {
            address: *implementation,
            nonce: *nonce,
            chain_id: U256::from(ccx.ecx.env.cfg.chain_id),
        };
        let signed_auth = SignedAuthorization::new_unchecked(
            auth,
            *v,
            U256::from_be_bytes(r.0),
            U256::from_be_bytes(s.0),
        );
        write_delegation(ccx, signed_auth.clone())?;
        ccx.state.active_delegation = Some(signed_auth);
        Ok(Default::default())
    }
}

impl Cheatcode for signDelegationCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { implementation, privateKey } = self;
        let signer = PrivateKeySigner::from_bytes(&B256::from(*privateKey))?;
        let authority = signer.address();
        let (auth, nonce) = create_auth(ccx, *implementation, authority)?;
        let sig = signer.sign_hash_sync(&auth.signature_hash())?;
        Ok(sig_to_delegation(sig, nonce, *implementation).abi_encode())
    }
}

impl Cheatcode for signAndAttachDelegationCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { implementation, privateKey } = self;
        let signer = PrivateKeySigner::from_bytes(&B256::from(*privateKey))?;
        let authority = signer.address();
        let (auth, nonce) = create_auth(ccx, *implementation, authority)?;
        let sig = signer.sign_hash_sync(&auth.signature_hash())?;
        let signed_auth = sig_to_auth(sig, auth);
        write_delegation(ccx, signed_auth.clone())?;
        ccx.state.active_delegation = Some(signed_auth);
        Ok(sig_to_delegation(sig, nonce, *implementation).abi_encode())
    }
}

fn create_auth(
    ccx: &mut CheatsCtxt,
    implementation: Address,
    authority: Address,
) -> Result<(Authorization, u64)> {
    let authority_acc = ccx.ecx.journaled_state.load_account(authority, &mut ccx.ecx.db)?;
    let nonce = authority_acc.data.info.nonce;
    Ok((
        Authorization {
            address: implementation,
            nonce,
            chain_id: U256::from(ccx.ecx.env.cfg.chain_id),
        },
        nonce,
    ))
}

fn write_delegation(ccx: &mut CheatsCtxt, auth: SignedAuthorization) -> Result<()> {
    let authority = auth.recover_authority().map_err(|e| format!("{e}"))?;
    let authority_acc = ccx.ecx.journaled_state.load_account(authority, &mut ccx.ecx.db)?;
    if authority_acc.data.info.nonce != auth.nonce {
        return Err("invalid nonce".into());
    }
    authority_acc.data.info.nonce += 1;
    let bytecode = Bytecode::new_eip7702(*auth.address());
    ccx.ecx.journaled_state.set_code(authority, bytecode);
    Ok(())
}

fn sig_to_delegation(
    sig: PrimitiveSignature,
    nonce: u64,
    implementation: Address,
) -> SignedDelegation {
    SignedDelegation {
        v: sig.v() as u8,
        r: sig.r().into(),
        s: sig.s().into(),
        nonce,
        implementation,
    }
}

fn sig_to_auth(sig: PrimitiveSignature, auth: Authorization) -> SignedAuthorization {
    SignedAuthorization::new_unchecked(auth, sig.v() as u8, sig.r(), sig.s())
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

impl Cheatcode for getWalletsCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let wallets = ccx.state.wallets().signers().unwrap_or_default();
        Ok(wallets.abi_encode())
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
pub struct WalletsInner {
    /// All signers in scope of the script.
    pub multi_wallet: MultiWallet,
    /// Optional signer provided as `--sender` flag.
    pub provided_sender: Option<Address>,
}

/// Clonable wrapper around [`WalletsInner`].
#[derive(Debug, Clone)]
pub struct Wallets {
    /// Inner data.
    pub inner: Arc<Mutex<WalletsInner>>,
}

impl Wallets {
    #[allow(missing_docs)]
    pub fn new(multi_wallet: MultiWallet, provided_sender: Option<Address>) -> Self {
        Self { inner: Arc::new(Mutex::new(WalletsInner { multi_wallet, provided_sender })) }
    }

    /// Consumes [Wallets] and returns [MultiWallet].
    ///
    /// Panics if [Wallets] is still in use.
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

    /// Number of signers in the [MultiWallet].
    pub fn len(&self) -> usize {
        let mut inner = self.inner.lock();
        let signers = inner.multi_wallet.signers();
        if signers.is_err() {
            return 0;
        }
        signers.unwrap().len()
    }

    /// Whether the [MultiWallet] is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Sets up broadcasting from a script using `new_origin` as the sender.
fn broadcast(ccx: &mut CheatsCtxt, new_origin: Option<&Address>, single_call: bool) -> Result {
    let depth = ccx.ecx.journaled_state.depth();
    ensure!(
        ccx.state.get_prank(depth).is_none(),
        "you have an active prank; broadcasting and pranks are not compatible"
    );
    ensure!(ccx.state.broadcast.is_none(), "a broadcast is active already");

    let mut new_origin = new_origin.cloned();

    if new_origin.is_none() {
        let mut wallets = ccx.state.wallets().inner.lock();
        if let Some(provided_sender) = wallets.provided_sender {
            new_origin = Some(provided_sender);
        } else {
            let signers = wallets.multi_wallet.signers()?;
            if signers.len() == 1 {
                let address = signers.keys().next().unwrap();
                new_origin = Some(*address);
            }
        }
    }

    let broadcast = Broadcast {
        new_origin: new_origin.unwrap_or(ccx.ecx.env.tx.caller),
        original_caller: ccx.caller,
        original_origin: ccx.ecx.env.tx.caller,
        depth,
        single_call,
    };
    debug!(target: "cheatcodes", ?broadcast, "started");
    ccx.state.broadcast = Some(broadcast);
    Ok(Default::default())
}

/// Sets up broadcasting from a script with the sender derived from `private_key`.
/// Adds this private key to `state`'s `wallets` vector to later be used for signing
/// if broadcast is successful.
fn broadcast_key(ccx: &mut CheatsCtxt, private_key: &U256, single_call: bool) -> Result {
    let wallet = super::crypto::parse_wallet(private_key)?;
    let new_origin = wallet.address();

    let result = broadcast(ccx, Some(&new_origin), single_call);
    if result.is_ok() {
        let wallets = ccx.state.wallets();
        wallets.add_local_signer(wallet);
    }
    result
}
