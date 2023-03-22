//! Support for "cheat codes" / bypass functions

use anvil_core::eth::transaction::IMPERSONATED_SIGNATURE;
use ethers::types::{Address, Signature};
use forge::hashbrown::HashSet;
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
        self.state.read().impersonated_accounts.contains(&addr)
    }

    /// Returns the signature to use to bypass transaction signing
    pub fn bypass_signature(&self) -> Signature {
        self.state.read().bypass_signature
    }
}

/// Container type for all the state variables
#[derive(Debug, Clone)]
pub struct CheatsState {
    /// All accounts that are currently impersonated
    pub impersonated_accounts: HashSet<Address>,
    /// The signature used for the `eth_sendUnsignedTransaction` cheat code
    pub bypass_signature: Signature,
}

impl Default for CheatsState {
    fn default() -> Self {
        Self { impersonated_accounts: Default::default(), bypass_signature: IMPERSONATED_SIGNATURE }
    }
}
