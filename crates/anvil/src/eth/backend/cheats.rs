//! Support for "cheat codes" / bypass functions

use alloy_evm::precompiles::{Precompile, PrecompileInput};
use alloy_primitives::{
    Address, Bytes,
    map::{AddressHashSet, foldhash::HashMap},
};
use parking_lot::RwLock;
use revm::precompile::{
    PrecompileError, PrecompileId, PrecompileOutput, PrecompileResult, secp256k1::ec_recover_run,
    utilities::right_pad,
};
use std::{borrow::Cow, sync::Arc};

/// ID for the [`CheatEcrecover::precompile_id`] precompile.
static PRECOMPILE_ID_CHEAT_ECRECOVER: PrecompileId =
    PrecompileId::Custom(Cow::Borrowed("cheat_ecrecover"));

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
        trace!(target: "cheats", %addr, "start impersonating");
        // When somebody **explicitly** impersonates an account we need to store it so we are able
        // to return it from `eth_accounts`. That's why we do not simply call `is_impersonated()`
        // which does not check that list when auto impersonation is enabled.
        !self.state.write().impersonated_accounts.insert(addr)
    }

    /// Removes the account that from the impersonated set
    pub fn stop_impersonating(&self, addr: &Address) {
        trace!(target: "cheats", %addr, "stop impersonating");
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

    fn precompile_id(&self) -> &PrecompileId {
        &PRECOMPILE_ID_CHEAT_ECRECOVER
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impersonate_returns_false_then_true() {
        let mgr = CheatsManager::default();
        let addr = Address::from([1u8; 20]);
        assert!(!mgr.impersonate(addr));
        assert!(mgr.impersonate(addr));
    }
}
