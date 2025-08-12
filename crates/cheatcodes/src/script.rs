//! Implementations of [`Scripting`](spec::Group::Scripting) cheatcodes.

use crate::{Cheatcode, CheatsCtxt, Result, Vm::*};
use alloy_consensus::{SidecarBuilder, SimpleCoder};
use alloy_primitives::{Address, B256, U256, Uint};
use alloy_rpc_types::Authorization;
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::SolValue;
use foundry_wallets::{WalletSigner, multi_wallet::MultiWallet};
use parking_lot::Mutex;
use revm::{
    bytecode::Bytecode,
    context::JournalTr,
    context_interface::transaction::SignedAuthorization,
    primitives::{KECCAK_EMPTY, hardfork::SpecId},
};
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

impl Cheatcode for attachDelegation_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { signedDelegation } = self;
        attach_delegation(ccx, signedDelegation, false)
    }
}

impl Cheatcode for attachDelegation_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { signedDelegation, crossChain } = self;
        attach_delegation(ccx, signedDelegation, *crossChain)
    }
}

impl Cheatcode for signDelegation_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { implementation, privateKey } = *self;
        sign_delegation(ccx, privateKey, implementation, None, false, false)
    }
}

impl Cheatcode for signDelegation_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { implementation, privateKey, nonce } = *self;
        sign_delegation(ccx, privateKey, implementation, Some(nonce), false, false)
    }
}

impl Cheatcode for signDelegation_2Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { implementation, privateKey, crossChain } = *self;
        sign_delegation(ccx, privateKey, implementation, None, crossChain, false)
    }
}

impl Cheatcode for signAndAttachDelegation_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { implementation, privateKey } = *self;
        sign_delegation(ccx, privateKey, implementation, None, false, true)
    }
}

impl Cheatcode for signAndAttachDelegation_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { implementation, privateKey, nonce } = *self;
        sign_delegation(ccx, privateKey, implementation, Some(nonce), false, true)
    }
}

impl Cheatcode for signAndAttachDelegation_2Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { implementation, privateKey, crossChain } = *self;
        sign_delegation(ccx, privateKey, implementation, None, crossChain, true)
    }
}

/// Helper function to attach an EIP-7702 delegation.
fn attach_delegation(
    ccx: &mut CheatsCtxt,
    delegation: &SignedDelegation,
    cross_chain: bool,
) -> Result {
    let SignedDelegation { v, r, s, nonce, implementation } = delegation;
    // Set chain id to 0 if universal deployment is preferred.
    // See https://github.com/ethereum/EIPs/blob/master/EIPS/eip-7702.md#protection-from-malleability-cross-chain
    let chain_id = if cross_chain { U256::from(0) } else { U256::from(ccx.ecx.cfg.chain_id) };

    let auth = Authorization { address: *implementation, nonce: *nonce, chain_id };
    let signed_auth = SignedAuthorization::new_unchecked(
        auth,
        *v,
        U256::from_be_bytes(r.0),
        U256::from_be_bytes(s.0),
    );
    write_delegation(ccx, signed_auth.clone())?;
    ccx.state.add_delegation(signed_auth);
    Ok(Default::default())
}

/// Helper function to sign and attach (if needed) an EIP-7702 delegation.
/// Uses the provided nonce, otherwise retrieves and increments the nonce of the EOA.
fn sign_delegation(
    ccx: &mut CheatsCtxt,
    private_key: Uint<256, 4>,
    implementation: Address,
    nonce: Option<u64>,
    cross_chain: bool,
    attach: bool,
) -> Result<Vec<u8>> {
    let signer = PrivateKeySigner::from_bytes(&B256::from(private_key))?;
    let nonce = if let Some(nonce) = nonce {
        nonce
    } else {
        let authority_acc = ccx.ecx.journaled_state.load_account(signer.address())?;
        // Calculate next nonce considering existing active delegations
        next_delegation_nonce(
            &ccx.state.active_delegations,
            signer.address(),
            &ccx.state.broadcast,
            authority_acc.data.info.nonce,
        )
    };
    let chain_id = if cross_chain { U256::from(0) } else { U256::from(ccx.ecx.cfg.chain_id) };

    let auth = Authorization { address: implementation, nonce, chain_id };
    let sig = signer.sign_hash_sync(&auth.signature_hash())?;
    // Attach delegation.
    if attach {
        let signed_auth = SignedAuthorization::new_unchecked(auth, sig.v() as u8, sig.r(), sig.s());
        write_delegation(ccx, signed_auth.clone())?;
        ccx.state.add_delegation(signed_auth);
    }
    Ok(SignedDelegation {
        v: sig.v() as u8,
        r: sig.r().into(),
        s: sig.s().into(),
        nonce,
        implementation,
    }
    .abi_encode())
}

/// Returns the next valid nonce for a delegation, considering existing active delegations.
fn next_delegation_nonce(
    active_delegations: &[SignedAuthorization],
    authority: Address,
    broadcast: &Option<Broadcast>,
    account_nonce: u64,
) -> u64 {
    match active_delegations
        .iter()
        .rfind(|auth| auth.recover_authority().is_ok_and(|recovered| recovered == authority))
    {
        Some(auth) => {
            // Increment nonce of last recorded delegation.
            auth.nonce + 1
        }
        None => {
            // First time a delegation is added for this authority.
            if let Some(broadcast) = broadcast {
                // Increment nonce if authority is the sender of transaction.
                if broadcast.new_origin == authority {
                    return account_nonce + 1;
                }
            }
            // Return current nonce if authority is not the sender of transaction.
            account_nonce
        }
    }
}

fn write_delegation(ccx: &mut CheatsCtxt, auth: SignedAuthorization) -> Result<()> {
    let authority = auth.recover_authority().map_err(|e| format!("{e}"))?;
    let authority_acc = ccx.ecx.journaled_state.load_account(authority)?;

    let expected_nonce = next_delegation_nonce(
        &ccx.state.active_delegations,
        authority,
        &ccx.state.broadcast,
        authority_acc.data.info.nonce,
    );

    if expected_nonce != auth.nonce {
        return Err(format!(
            "invalid nonce for {authority:?}: expected {expected_nonce}, got {}",
            auth.nonce
        )
        .into());
    }

    if auth.address.is_zero() {
        // Set empty code if the delegation address of authority is 0x.
        // See https://github.com/ethereum/EIPs/blob/master/EIPS/eip-7702.md#behavior.
        ccx.ecx.journaled_state.set_code_with_hash(authority, Bytecode::default(), KECCAK_EMPTY);
    } else {
        let bytecode = Bytecode::new_eip7702(*auth.address());
        ccx.ecx.journaled_state.set_code(authority, bytecode);
    }
    Ok(())
}

impl Cheatcode for attachBlobCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { blob } = self;
        ensure!(
            ccx.ecx.cfg.spec >= SpecId::CANCUN,
            "`attachBlob` is not supported before the Cancun hard fork; \
             see EIP-4844: https://eips.ethereum.org/EIPS/eip-4844"
        );
        let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(blob);
        let sidecar = sidecar.build().map_err(|e| format!("{e}"))?;
        ccx.state.active_blob_sidecar = Some(sidecar);
        Ok(Default::default())
    }
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
    pub depth: usize,
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

/// Cloneable wrapper around [`WalletsInner`].
#[derive(Debug, Clone)]
pub struct Wallets {
    /// Inner data.
    pub inner: Arc<Mutex<WalletsInner>>,
}

impl Wallets {
    #[expect(missing_docs)]
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
        Ok(self.inner.lock().multi_wallet.signers()?.keys().copied().collect())
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

    let mut new_origin = new_origin.copied();

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
        new_origin: new_origin.unwrap_or(ccx.ecx.tx.caller),
        original_caller: ccx.caller,
        original_origin: ccx.ecx.tx.caller,
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
