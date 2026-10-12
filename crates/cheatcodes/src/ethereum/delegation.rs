//! Script EIP-7702 authorization signing and local delegation state.

use super::{ApplyError, Cheatcodes};
use crate::{Error, Vm::SignedDelegation, script::next_delegation_nonce};
use alloy_eips::eip7702::{Authorization, SignedAuthorization};
use alloy_primitives::{Address, Bytes, U256};
use alloy_signer::SignerSync;
use alloy_sol_types::SolValue;
use evm2::{bytecode::Bytecode, interpreter::Interpreter};
use foundry_evm_core::ethereum::FoundryEvmTypes;

impl Cheatcodes {
    pub(super) fn sign_delegation(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        private_key: U256,
        implementation: Address,
        nonce: Option<u64>,
        cross_chain: bool,
        attach: bool,
    ) -> Result<Bytes, ApplyError> {
        let signer = crate::wallet::parse_wallet(&private_key)?;
        let host = interp.host();
        let account_nonce = host
            .state_mut()
            .account_info_untracked(&signer.address())?
            .map_or(0, |info| info.nonce);
        let nonce = nonce.unwrap_or_else(|| {
            next_delegation_nonce(
                &self.active_delegations,
                signer.address(),
                &self.broadcast,
                account_nonce,
            )
        });
        let chain_id = if cross_chain {
            U256::ZERO
        } else {
            U256::from(host.ext().chain_id_override.unwrap_or(host.version().chain_id))
        };
        let auth = Authorization { address: implementation, nonce, chain_id };
        let sig = signer
            .sign_hash_sync(&auth.signature_hash())
            .map_err(|error| Error::from(error.to_string()))?;
        let delegation = SignedDelegation {
            v: sig.v() as u8,
            r: sig.r().into(),
            s: sig.s().into(),
            nonce,
            implementation,
        };
        if attach {
            self.attach_delegation(interp, &delegation, cross_chain)?;
        }
        Ok(delegation.abi_encode().into())
    }

    pub(super) fn attach_delegation(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        delegation: &SignedDelegation,
        cross_chain: bool,
    ) -> Result<(), ApplyError> {
        let host = interp.host();
        let chain_id = if cross_chain {
            U256::ZERO
        } else {
            U256::from(host.ext().chain_id_override.unwrap_or(host.version().chain_id))
        };
        let auth = SignedAuthorization::new_unchecked(
            Authorization { address: delegation.implementation, nonce: delegation.nonce, chain_id },
            delegation.v,
            delegation.r.into(),
            delegation.s.into(),
        );
        let authority = auth.recover_authority().map_err(|error| Error::from(error.to_string()))?;
        let account_nonce =
            host.state_mut().account_info_untracked(&authority)?.map_or(0, |info| info.nonce);
        let expected = next_delegation_nonce(
            &self.active_delegations,
            authority,
            &self.broadcast,
            account_nonce,
        );
        if expected != auth.nonce() {
            return Err(Error::from(format!(
                "invalid nonce for {authority:?}: expected {expected}, got {}",
                auth.nonce()
            ))
            .into());
        }
        let code = if delegation.implementation.is_zero() {
            Bytecode::default()
        } else {
            Bytecode::new_eip7702(delegation.implementation)
        };
        // Attachment changes local code; the collected transaction consumes the nonce.
        host.state_mut().account(&authority)?.set_code_slow(code);
        self.active_delegations.push(auth);
        Ok(())
    }
}
