//! Support for "cheat codes" / bypass functions

use alloy_evm::precompiles::{Precompile, PrecompileInput};
use alloy_primitives::{
    Address, Bytes,
    map::{AddressHashSet, foldhash::HashMap},
};
use alloy_signer::Signature;
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

/// A custom ecrecover precompile that supports cheat-based signature overrides.
#[derive(Clone, Debug)]
pub struct CheatEcrecover {
    cheats: Arc<CheatsManager>,
}

impl CheatEcrecover {
    pub fn new(cheats: Arc<CheatsManager>) -> Self {
        Self { cheats }
    }
}

impl Precompile for CheatEcrecover {
    fn call(&self, input: PrecompileInput<'_>) -> PrecompileResult {
        const ECRECOVER_BASE: u64 = 3_000;

        if input.gas < ECRECOVER_BASE {
            return Err(PrecompileError::OutOfGas);
        }

        // Fast path: if no overrides are present, call real ecrecover
        if !self.cheats.has_recover_overrides() {
            return ec_recover_run(input.data, input.gas);
        }

        let padded_input = right_pad::<128>(input.data);

        // Validate recovery ID: only v = 27 or 28 allowed
        if !(padded_input[32..63].iter().all(|&b| b == 0) && matches!(padded_input[63], 27 | 28)) {
            return Ok(PrecompileOutput::new(ECRECOVER_BASE, Bytes::new()));
        }

        // Construct signature bytes
        let sig_bytes: [u8; 65] = {
            let mut buf = [0u8; 65];
            buf[..64].copy_from_slice(&padded_input[64..128]);
            buf[64] = padded_input[63];
            buf
        };

        // Parse signature
        let sig = match Signature::try_from(&sig_bytes[..]) {
            Ok(sig) => sig,
            Err(_) => return Ok(PrecompileOutput::new(ECRECOVER_BASE, Bytes::new())),
        };

        // Check for override
        if let Some(addr) =
            self.cheats.get_recover_override(&Bytes::copy_from_slice(&sig.as_bytes()))
        {
            let mut out = [0u8; 32];
            out[12..].copy_from_slice(addr.as_slice()); // Right-align the address
            return Ok(PrecompileOutput::new(ECRECOVER_BASE, Bytes::copy_from_slice(&out)));
        }

        // Fallback to native ecrecover
        ec_recover_run(input.data, input.gas)
    }

    fn is_pure(&self) -> bool {
        false
    }
}
