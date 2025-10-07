//! Support for "cheat codes" / bypass functions
//! Same logic as for anvil, except the `H160` usage over `Address`.

use std::collections::HashSet;
use subxt::utils::H160;

/// Manages user modifications that may affect the node's behavior
///
/// Contains the state of executed, non-eth standard cheat code RPC
#[derive(Clone, Debug, Default)]
pub(crate) struct ImpersonationManager {
    /// All accounts that are currently impersonated
    pub impersonated_accounts: HashSet<H160>,
    /// If set to true will make the `is_impersonated` function always return true
    pub auto_impersonate_accounts: bool,
}

impl ImpersonationManager {
    /// Sets the account to impersonate
    ///
    /// This also accepts the actual code hash if the address is a contract to bypass EIP-3607
    ///
    /// Returns `true` if the account is already impersonated
    pub fn impersonate(&mut self, addr: H160) -> bool {
        trace!(target: "cheats", "Start impersonating {:?}", addr);
        // When somebody **explicitly** impersonates an account we need to store it so we are able
        // to return it from `eth_accounts`. That's why we do not simply call `is_impersonated()`
        // which does not check that list when auto impersonation is enabled.
        if self.impersonated_accounts.contains(&addr) {
            // need to check if already impersonated, so we don't overwrite the code
            return true;
        }
        self.impersonated_accounts.insert(addr)
    }

    /// Removes the account that from the impersonated set
    pub fn stop_impersonating(&mut self, addr: &H160) {
        trace!(target: "cheats", "Stop impersonating {:?}", addr);
        self.impersonated_accounts.remove(addr);
    }

    /// Returns true if the `addr` is currently impersonated
    pub fn is_impersonated(&self, addr: H160) -> bool {
        if self.auto_impersonate_accounts() {
            true
        } else {
            self.impersonated_accounts.contains(&addr)
        }
    }

    /// Returns true is auto impersonation is enabled
    pub fn auto_impersonate_accounts(&self) -> bool {
        self.auto_impersonate_accounts
    }

    /// Sets the auto impersonation flag which if set to true will make the `is_impersonated`
    /// function always return true
    pub fn set_auto_impersonate_account(&mut self, enabled: bool) {
        trace!(target: "cheats", "Auto impersonation set to {:?}", enabled);
        self.auto_impersonate_accounts = enabled
    }
}
