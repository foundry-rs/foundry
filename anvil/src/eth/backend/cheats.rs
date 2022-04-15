//! Support for "cheat codes" / bypass functions

use ethers::types::Address;
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
}

/// Container type for all the state variables
#[derive(Debug, Clone, Default)]
pub struct CheatsState {
    /// The account that's currently impersonated
    pub impersonated_account: Option<Address>,
}
