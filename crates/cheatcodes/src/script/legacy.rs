//! Legacy engine implementations of scripting cheatcodes.

use super::{Broadcast, build_blob_sidecar, next_delegation_nonce};
use crate::{Cheatcode, CheatcodesCore, CheatsCtxt, Result, Vm::*, evm::journaled_account};
use alloy_primitives::{Address, B256, U256};
use alloy_rpc_types::Authorization;
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::SolValue;
use foundry_evm_core::evm::FoundryEvmNetwork;
use revm::{
    bytecode::Bytecode,
    context::{ContextTr, JournalTr},
    context_interface::transaction::SignedAuthorization,
    primitives::{KECCAK_EMPTY, hardfork::SpecId},
};

impl Cheatcode for broadcast_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        broadcast(ccx, None, true)
    }
}

impl Cheatcode for broadcast_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { signer } = self;
        broadcast(ccx, Some(signer), true)
    }
}

impl Cheatcode for broadcast_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { privateKey } = self;
        broadcast_key(ccx, privateKey, true)
    }
}

impl Cheatcode for attachDelegation_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { signedDelegation } = self;
        attach_delegation(ccx, signedDelegation, false)
    }
}

impl Cheatcode for attachDelegation_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { signedDelegation, crossChain } = self;
        attach_delegation(ccx, signedDelegation, *crossChain)
    }
}

impl Cheatcode for signDelegation_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { implementation, privateKey } = *self;
        sign_delegation(ccx, privateKey, implementation, None, false, false)
    }
}

impl Cheatcode for signDelegation_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { implementation, privateKey, nonce } = *self;
        sign_delegation(ccx, privateKey, implementation, Some(nonce), false, false)
    }
}

impl Cheatcode for signDelegation_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { implementation, privateKey, crossChain } = *self;
        sign_delegation(ccx, privateKey, implementation, None, crossChain, false)
    }
}

impl Cheatcode for signAndAttachDelegation_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { implementation, privateKey } = *self;
        sign_delegation(ccx, privateKey, implementation, None, false, true)
    }
}

impl Cheatcode for signAndAttachDelegation_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { implementation, privateKey, nonce } = *self;
        sign_delegation(ccx, privateKey, implementation, Some(nonce), false, true)
    }
}

impl Cheatcode for signAndAttachDelegation_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { implementation, privateKey, crossChain } = *self;
        sign_delegation(ccx, privateKey, implementation, None, crossChain, true)
    }
}

/// Helper function to attach an EIP-7702 delegation.
fn attach_delegation<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    delegation: &SignedDelegation,
    cross_chain: bool,
) -> Result {
    let SignedDelegation { v, r, s, nonce, implementation } = delegation;
    // Set chain id to 0 if universal deployment is preferred.
    // See https://github.com/ethereum/EIPs/blob/master/EIPS/eip-7702.md#protection-from-malleability-cross-chain
    let chain_id = if cross_chain { U256::ZERO } else { U256::from(ccx.chain_id()) };

    let auth = Authorization { address: *implementation, nonce: *nonce, chain_id };
    let signed_auth = SignedAuthorization::new_unchecked(auth, *v, (*r).into(), (*s).into());
    write_delegation(ccx, signed_auth.clone())?;
    ccx.state.add_delegation(signed_auth);
    Ok(Default::default())
}

/// Helper function to sign and attach (if needed) an EIP-7702 delegation.
/// Uses the provided nonce, otherwise retrieves and increments the nonce of the EOA.
fn sign_delegation<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    private_key: U256,
    implementation: Address,
    nonce: Option<u64>,
    cross_chain: bool,
    attach: bool,
) -> Result<Vec<u8>> {
    let signer = PrivateKeySigner::from_bytes(&B256::from(private_key))?;
    let nonce = if let Some(nonce) = nonce {
        nonce
    } else {
        let account_nonce = {
            let authority_acc = ccx.ecx.journal_mut().load_account(signer.address())?;
            authority_acc.info.nonce
        };
        // Calculate next nonce considering existing active delegations
        next_delegation_nonce(
            &ccx.state.active_delegations,
            signer.address(),
            &ccx.state.broadcast,
            account_nonce,
        )
    };
    let chain_id = if cross_chain { U256::ZERO } else { U256::from(ccx.chain_id()) };

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

fn write_delegation<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    auth: SignedAuthorization,
) -> Result<()> {
    let authority = auth.recover_authority().map_err(|e| format!("{e}"))?;
    let account_nonce = {
        let authority_acc = ccx.ecx.journal_mut().load_account(authority)?;
        authority_acc.info.nonce
    };

    let expected_nonce = next_delegation_nonce(
        &ccx.state.active_delegations,
        authority,
        &ccx.state.broadcast,
        account_nonce,
    );

    if expected_nonce != auth.nonce() {
        return Err(format!(
            "invalid nonce for {authority:?}: expected {expected_nonce}, got {}",
            auth.nonce()
        )
        .into());
    }

    if auth.address.is_zero() {
        // Set empty code if the delegation address of authority is 0x.
        // See https://github.com/ethereum/EIPs/blob/master/EIPS/eip-7702.md#behavior.
        ccx.ecx.journal_mut().set_code_with_hash(authority, Bytecode::default(), KECCAK_EMPTY);
    } else {
        let bytecode = Bytecode::new_eip7702(*auth.address());
        ccx.ecx.journal_mut().set_code(authority, bytecode);
    }
    Ok(())
}

impl Cheatcode for attachBlobCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { blob } = self;
        ensure!(
            ccx.spec().into() >= SpecId::CANCUN,
            "`attachBlob` is not supported before the Cancun hard fork; \
             see EIP-4844: https://eips.ethereum.org/EIPS/eip-4844"
        );
        ccx.state.active_blob_sidecar =
            Some(build_blob_sidecar(blob, ccx.spec().into() >= SpecId::OSAKA)?);
        Ok(Default::default())
    }
}

impl Cheatcode for startBroadcast_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        broadcast(ccx, None, false)
    }
}

impl Cheatcode for startBroadcast_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { signer } = self;
        broadcast(ccx, Some(signer), false)
    }
}

impl Cheatcode for startBroadcast_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { privateKey } = self;
        broadcast_key(ccx, privateKey, false)
    }
}

impl Cheatcode for stopBroadcastCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        let Some(broadcast) = ccx.state.broadcast.take() else {
            bail!("no broadcast in progress to stop");
        };
        debug!(target: "cheatcodes", ?broadcast, "stopped");
        Ok(Default::default())
    }
}

impl Cheatcode for getWalletsCall {
    fn apply_context(&self, state: &mut CheatcodesCore) -> Result {
        let wallets = state.wallets().signers().unwrap_or_default();
        Ok(wallets.abi_encode())
    }
}

/// Sets up broadcasting from a script using `new_origin` as the sender.
fn broadcast<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    new_origin: Option<&Address>,
    single_call: bool,
) -> Result {
    let depth = ccx.depth();
    ensure!(
        ccx.state.get_prank(depth).is_none(),
        "you have an active prank; broadcasting and pranks are not compatible"
    );
    ensure!(ccx.state.broadcast.is_none(), "a broadcast is active already");

    let mut new_origin = new_origin.copied();

    if new_origin.is_none() {
        let mut wallets = ccx.state.core.wallets().inner.lock();
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
    let new_origin = new_origin.unwrap_or(ccx.tx_caller());
    // Ensure new origin is loaded and touched.
    let _ = journaled_account(ccx.ecx, new_origin)?;

    let broadcast = Broadcast {
        new_origin,
        original_caller: ccx.caller,
        original_origin: ccx.tx_caller(),
        depth,
        single_call,
        deploy_from_code: false,
    };
    debug!(target: "cheatcodes", ?broadcast, "started");
    ccx.state.broadcast = Some(broadcast);
    Ok(Default::default())
}

/// Sets up broadcasting from a script with the sender derived from `private_key`.
/// Adds this private key to `state`'s `wallets` vector to later be used for signing
/// if broadcast is successful.
fn broadcast_key<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    private_key: &U256,
    single_call: bool,
) -> Result {
    let wallet = crate::wallet::parse_wallet(private_key)?;
    let new_origin = wallet.address();

    let result = broadcast(ccx, Some(&new_origin), single_call);
    if result.is_ok() {
        let wallets = ccx.state.core.wallets();
        wallets.add_local_signer(wallet);
    }
    result
}
