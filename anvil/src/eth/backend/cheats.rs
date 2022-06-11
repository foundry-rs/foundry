//! Support for "cheat codes" / bypass functions

use anvil_core::eth::transaction::TypedTransaction;
use ethers::types::{Address, Signature, U256};
use parking_lot::RwLock;
use std::{collections::HashSet, sync::Arc};
use tracing::trace;

/// The signature used to bypass signing via the `eth_sendUnsignedTransaction` cheat RPC
const BYPASS_SIGNATURE: Signature =
    Signature { r: U256([0, 0, 0, 0]), s: U256([0, 0, 0, 0]), v: 0 };

/// Returns `true` if the signature of the `transaction` is the `BYPASS_SIGNATURE`
pub fn is_bypassed(transaction: &TypedTransaction) -> bool {
    transaction.signature() == BYPASS_SIGNATURE
}

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
    /// Returns `true` if the account is already impersonated
    pub fn impersonate(&self, addr: Address) -> bool {
        trace!(target: "cheats", "Start impersonating {:?}", addr);
        self.state.write().impersonated_account.insert(addr)
    }

    /// Removes the account that from the impersonated set
    pub fn stop_impersonating(&self, addr: &Address) {
        trace!(target: "cheats", "Stop impersonating {:?}", addr);
        self.state.write().impersonated_account.remove(addr);
    }

    /// Returns true if the `addr` is currently impersonated
    pub fn is_impersonated(&self, addr: Address) -> bool {
        self.state.read().impersonated_account.contains(&addr)
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
    pub impersonated_account: HashSet<Address>,
    /// The signature used for the `eth_sendUnsignedTransaction` cheat code
    pub bypass_signature: Signature,
}

impl Default for CheatsState {
    fn default() -> Self {
        Self { impersonated_account: Default::default(), bypass_signature: BYPASS_SIGNATURE }
    }
}
