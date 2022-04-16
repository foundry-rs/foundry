//! Support for "cheat codes" / bypass functions

use ethers::types::{Address, Signature, U256};
use parking_lot::RwLock;
use std::sync::Arc;
use tracing::trace;

/// The signature used to bypass signing via the `eth_sendUnsignedTransaction` cheat RPC
const BYPASS_SIGNATURE: Signature =
    Signature { r: U256([0, 0, 0, 0]), s: U256([0, 0, 0, 0]), v: 0 };

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
    /// Sets the account to impersonate and returns the account that was previously impersonated if
    /// any
    pub fn impersonate(&self, account: Address) -> Option<Address> {
        trace!(target: "cheats", "Start impersonating {:?}", account);
        self.state.write().impersonated_account.replace(account)
    }

    /// Removes the account that is currently impersonated, if any
    pub fn stop_impersonating(&self) -> Option<Address> {
        let acc = self.state.write().impersonated_account.take();
        if let Some(ref acc) = acc {
            trace!(target: "cheats", "Stop impersonating {:?}", acc);
        }
        acc
    }

    /// Returns the account that's currently being impersonated
    pub fn impersonated_account(&self) -> Option<Address> {
        self.state.read().impersonated_account
    }

    /// Returns the signature to use to bypass transaction signing
    pub fn bypass_signature(&self) -> Signature {
        self.state.read().bypass_signature
    }
}

/// Container type for all the state variables
#[derive(Debug, Clone)]
pub struct CheatsState {
    /// The account that's currently impersonated
    pub impersonated_account: Option<Address>,
    /// The signature used for the `eth_sendUnsignedTransaction` cheat code
    pub bypass_signature: Signature,
}

impl Default for CheatsState {
    fn default() -> Self {
        Self { impersonated_account: None, bypass_signature: BYPASS_SIGNATURE }
    }
}
