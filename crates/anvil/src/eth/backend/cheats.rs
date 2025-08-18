//! Support for "cheat codes" / bypass functions

use alloy_eips::eip7702::SignedAuthorization;
use alloy_evm::precompiles::{Precompile, PrecompileInput};
use alloy_primitives::{
    Address, Bytes, U256,
    map::{AddressHashSet, foldhash::HashMap},
};
use alloy_rpc_types::Authorization;
use parking_lot::RwLock;
use revm::precompile::{
    PrecompileError, PrecompileOutput, PrecompileResult, secp256k1::ec_recover_run,
    utilities::right_pad,
};
use std::sync::Arc;

/// Manages user modifications that may affect the node's behavior
///
/// Contains the state of executed, non-eth standard cheat code RPC
#[derive(Clone, Debug, Default)]
pub struct CheatsManager {
    /// shareable state
    state: Arc<RwLock<CheatsState>>,
}

impl CheatsManager {
    /// Sets the account to impersonate
    ///
    /// Returns `true` if the account is already impersonated
    pub fn impersonate(&self, addr: Address) -> bool {
        trace!(target: "cheats", "Start impersonating {:?}", addr);
        let mut state = self.state.write();
        // When somebody **explicitly** impersonates an account we need to store it so we are able
        // to return it from `eth_accounts`. That's why we do not simply call `is_impersonated()`
        // which does not check that list when auto impersonation is enabled.
        if state.impersonated_accounts.contains(&addr) {
            // need to check if already impersonated, so we don't overwrite the code
            return true;
        }
        state.impersonated_accounts.insert(addr)
    }

    /// Removes the account that from the impersonated set
    pub fn stop_impersonating(&self, addr: &Address) {
        trace!(target: "cheats", "Stop impersonating {:?}", addr);
        self.state.write().impersonated_accounts.remove(addr);
    }

    /// Returns true if the `addr` is currently impersonated
    pub fn is_impersonated(&self, addr: Address) -> bool {
        if self.auto_impersonate_accounts() {
            true
        } else {
            self.state.read().impersonated_accounts.contains(&addr)
        }
    }

    /// Returns true is auto impersonation is enabled
    pub fn auto_impersonate_accounts(&self) -> bool {
        self.state.read().auto_impersonate_accounts
    }

    /// Sets the auto impersonation flag which if set to true will make the `is_impersonated`
    /// function always return true
    pub fn set_auto_impersonate_account(&self, enabled: bool) {
        trace!(target: "cheats", "Auto impersonation set to {:?}", enabled);
        self.state.write().auto_impersonate_accounts = enabled
    }

    /// Returns all accounts that are currently being impersonated.
    pub fn impersonated_accounts(&self) -> AddressHashSet {
        self.state.read().impersonated_accounts.clone()
    }

    /// Registers an override so that `ecrecover(signature)` returns `addr`.
    pub fn add_recover_override(&self, sig: Bytes, addr: Address) {
        self.state.write().signature_overrides.insert(sig, addr);
    }

    /// If an override exists for `sig`, returns the address; otherwise `None`.
    pub fn get_recover_override(&self, sig: &Bytes) -> Option<Address> {
        self.state.read().signature_overrides.get(sig).copied()
    }

    /// Returns true if any ecrecover overrides have been registered.
    pub fn has_recover_overrides(&self) -> bool {
        !self.state.read().signature_overrides.is_empty()
    }

    /// Creates authorization entries for impersonated accounts with signature overrides.
    /// This allows impersonated accounts to be used in EIP-7702 transactions.
    pub fn create_impersonated_authorizations(
        &self,
        authorizations: &[SignedAuthorization],
        chain_id: u64,
    ) -> Vec<SignedAuthorization> {
        let mut authorization_list = authorizations.to_vec();
        for addr in self.impersonated_accounts() {
            let auth = Authorization { chain_id: U256::from(chain_id), address: addr, nonce: 0 };

            let signed_auth = SignedAuthorization::new_unchecked(
                auth,
                0,             // y_parity
                U256::from(1), // r
                U256::from(1), // s
            );

            let mut sig_bytes = [0u8; 65];
            let r_bytes = signed_auth.r().to_be_bytes::<32>();
            let s_bytes = signed_auth.s().to_be_bytes::<32>();
            sig_bytes[..32].copy_from_slice(&r_bytes);
            sig_bytes[32..64].copy_from_slice(&s_bytes);
            sig_bytes[64] = signed_auth.y_parity();
            let sig = Bytes::copy_from_slice(&sig_bytes);

            self.add_recover_override(sig, addr);
            authorization_list.push(signed_auth);
        }
        authorization_list
    }
}

/// Container type for all the state variables
#[derive(Clone, Debug, Default)]
pub struct CheatsState {
    /// All accounts that are currently impersonated
    pub impersonated_accounts: AddressHashSet,
    /// If set to true will make the `is_impersonated` function always return true
    pub auto_impersonate_accounts: bool,
    /// Overrides for ecrecover: Signature => Address
    pub signature_overrides: HashMap<Bytes, Address>,
}

impl CheatEcrecover {
    pub fn new(cheats: Arc<CheatsManager>) -> Self {
        Self { cheats }
    }
}

impl Precompile for CheatEcrecover {
    fn call(&self, input: PrecompileInput<'_>) -> PrecompileResult {
        if !self.cheats.has_recover_overrides() {
            return ec_recover_run(input.data, input.gas);
        }

        const ECRECOVER_BASE: u64 = 3_000;
        if input.gas < ECRECOVER_BASE {
            return Err(PrecompileError::OutOfGas);
        }
        let padded = right_pad::<128>(input.data);
        let v = padded[63];
        let mut sig_bytes = [0u8; 65];
        sig_bytes[..64].copy_from_slice(&padded[64..128]);
        sig_bytes[64] = v;
        let sig_bytes_wrapped = Bytes::copy_from_slice(&sig_bytes);
        if let Some(addr) = self.cheats.get_recover_override(&sig_bytes_wrapped) {
            let mut out = [0u8; 32];
            out[12..].copy_from_slice(addr.as_slice());
            return Ok(PrecompileOutput::new(ECRECOVER_BASE, Bytes::copy_from_slice(&out)));
        }
        ec_recover_run(input.data, input.gas)
    }

    fn is_pure(&self) -> bool {
        false
    }
}

/// A custom ecrecover precompile that supports cheat-based signature overrides.
#[derive(Clone, Debug)]
pub struct CheatEcrecover {
    cheats: Arc<CheatsManager>,
}
