//! Support for "cheat codes" / bypass functions

use anvil_core::eth::transaction::IMPERSONATED_SIGNATURE;
use ethers::types::{Address, Signature};
use foundry_evm::hashbrown::HashSet;
use parking_lot::RwLock;
use std::sync::Arc;
use tracing::trace;

/// Manages user modifications that may affect the node's behavior
///
/// Contains the state of executed, non-eth standard cheat code RPC
#[derive(Debug, Clone, Default)]
pub struct CheatsManager {
    /// shareable state
    state: Arc<RwLock<CheatsState>>,
}

// === impl CheatsManager ===

impl CheatsManager {
    /// Sets the account to impersonate
    ///
    /// This also accepts the actual code hash if the address is a contract to bypass EIP-3607
    ///
    /// Returns `true` if the account is already impersonated
    pub fn impersonate(&self, addr: Address) -> bool {
        trace!(target: "cheats", "Start impersonating {:?}", addr);
        let mut state = self.state.write();
        // When somebody **explicitly** impersonates an account we need to store it so we are able
        // to return it from `eth_accounts`. That's why we do not simply call `is_impersonated()`
        // which does not check that list when auto impersonation is enabeld.
        if state.impersonated_accounts.contains(&addr) {
            // need to check if already impersonated, so we don't overwrite the code
            return true
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
        if self.state.read().auto_impersonate_accounts {
            true
        } else {
            self.state.read().impersonated_accounts.contains(&addr)
        }
    }

    /// Returns the signature to use to bypass transaction signing
    pub fn bypass_signature(&self) -> Signature {
        self.state.read().bypass_signature
    }

    /// Sets the auto impersonation flag which if set to true will make the `is_impersonated`
    /// function always return true
    pub fn set_auto_impersonate_account(&self, enabled: bool) {
        trace!(target: "cheats", "Auto impersonation set to {:?}", enabled);
        self.state.write().auto_impersonate_accounts = enabled
    }

    /// Returns all accounts that are currently being impersonated.
    pub fn impersonated_accounts(&self) -> HashSet<Address> {
        self.state.read().impersonated_accounts.clone()
    }
}

/// Container type for all the state variables
#[derive(Debug, Clone)]
pub struct CheatsState {
    /// All accounts that are currently impersonated
    pub impersonated_accounts: HashSet<Address>,
    /// The signature used for the `eth_sendUnsignedTransaction` cheat code
    pub bypass_signature: Signature,
    /// If set to true will make the `is_impersonated` function always return true
    pub auto_impersonate_accounts: bool,
}

impl Default for CheatsState {
    fn default() -> Self {
        Self {
            impersonated_accounts: Default::default(),
            bypass_signature: IMPERSONATED_SIGNATURE,
            auto_impersonate_accounts: false,
        }
    }
}
