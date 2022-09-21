//! Support for "cheat codes" / bypass functions

use anvil_core::eth::transaction::TypedTransaction;
use ethers::types::{Address, Signature, H256, U256};
use forge::revm::Bytecode;
use parking_lot::RwLock;
use std::{collections::HashMap, sync::Arc};
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
    /// This also accepts the actual code hash if the address is a contract to bypass EIP-3607
    ///
    /// Returns `true` if the account is already impersonated
    pub fn impersonate(
        &self,
        addr: Address,
        code_hash: Option<H256>,
        code: Option<Bytecode>,
    ) -> bool {
        trace!(target: "cheats", "Start impersonating {:?}", addr);
        let mut state = self.state.write();
        if state.impersonated_accounts.contains_key(&addr) {
            // need to check if already impersonated so we don't overwrite the code
            return true
        }
        state
            .impersonated_accounts
            .insert(addr, ImpersonatedAccount::new(code_hash, code))
            .is_some()
    }

    /// Removes the account that from the impersonated set
    pub fn stop_impersonating(&self, addr: &Address) -> Option<(Option<H256>, Option<Bytecode>)> {
        trace!(target: "cheats", "Stop impersonating {:?}", addr);
        self.state.write().impersonated_accounts.remove(addr).map(|acc| (acc.code_hash, acc.code))
    }

    /// Returns true if the `addr` is currently impersonated
    pub fn is_impersonated(&self, addr: Address) -> bool {
        self.state.read().impersonated_accounts.contains_key(&addr)
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
    ///
    /// If the account is a contract it holds the hash of the contracts code that is temporarily
    /// set to `KECCAK_EMPTY` to bypass EIP-3607 which rejects transactions from senders with
    /// deployed code
    pub impersonated_accounts: HashMap<Address, ImpersonatedAccount>,
    /// The signature used for the `eth_sendUnsignedTransaction` cheat code
    pub bypass_signature: Signature,
}

impl Default for CheatsState {
    fn default() -> Self {
        Self { impersonated_accounts: Default::default(), bypass_signature: BYPASS_SIGNATURE }
    }
}

/// Tracks an impersonated account
#[derive(Debug, Clone)]
pub struct ImpersonatedAccount {
    /// The account's code hash
    pub code_hash: Option<H256>,
    /// The account's code
    pub code: Option<Bytecode>,
}

// === impl ImpersonatedAccount ===

impl ImpersonatedAccount {
    /// Create a new instance as `impersonated`
    pub fn new(code_hash: Option<H256>, code: Option<Bytecode>) -> Self {
        Self { code_hash, code }
    }
}
