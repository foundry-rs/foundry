//! Support for "cheat codes" / bypass functions

use alloy_evm::precompiles::{Precompile, PrecompileInput};
use alloy_primitives::{
    Address, B256, Bytes,
    map::{AddressHashSet, foldhash::HashMap},
};
use parking_lot::RwLock;
use revm::precompile::{
    PrecompileHalt, PrecompileId, PrecompileOutput, PrecompileResult, call_eth_precompile,
    secp256k1::ec_recover_run, utilities::right_pad,
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

    /// Sets the `prevrandao` value to use for the next mined block.
    ///
    /// This is a one-shot override that is consumed by the next block and applies to that block
    /// only.
    pub fn set_next_block_prevrandao(&self, prevrandao: B256) {
        trace!(target: "cheats", %prevrandao, "set next block prevrandao");
        self.state.write().next_block.prevrandao.replace(Some(prevrandao));
    }

    /// Sets the parent beacon block root to use for the next mined block.
    ///
    /// This is a one-shot override that is consumed by the next block and applies to that block
    /// only.
    pub fn set_next_block_parent_beacon_block_root(&self, root: B256) {
        trace!(target: "cheats", %root, "set next block parent beacon block root");
        self.state.write().next_block.parent_beacon_block_root.replace(Some(root));
    }

    /// Returns the manually set values for the next block without consuming them.
    pub(crate) fn next_block_overrides(&self) -> NextBlockOverrides {
        self.state.read().next_block
    }

    /// Takes the manually set `prevrandao` value for forced replay mining.
    pub fn take_next_block_prevrandao(&self) -> Option<B256> {
        self.state.write().next_block.prevrandao.replace(None)
    }

    /// Consumes the overrides a committed block was built with, keeping any set since.
    pub(crate) fn consume_next_block_overrides(&self, used: &NextBlockOverrides) {
        let mut state = self.state.write();
        state.next_block.prevrandao.consume(&used.prevrandao);
        state.next_block.parent_beacon_block_root.consume(&used.parent_beacon_block_root);
    }

    /// Restores the overrides saved with a state snapshot.
    pub(crate) fn restore_next_block_overrides(&self, saved: &NextBlockOverrides) {
        let mut state = self.state.write();
        state.next_block.prevrandao.replace(saved.prevrandao.value);
        state.next_block.parent_beacon_block_root.replace(saved.parent_beacon_block_root.value);
    }

    /// Clears any manually set values for the next block.
    ///
    /// Used on reset/revert so a set-but-unmined override does not leak into a later block,
    /// mirroring how the next-block timestamp override is cleared by `TimeManager::reset`.
    pub fn clear_next_block_overrides(&self) {
        self.restore_next_block_overrides(&NextBlockOverrides::default());
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
    /// Manually set values for the next mined block.
    pub(crate) next_block: NextBlockOverrides,
}

/// Values manually set for the next mined block.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NextBlockOverrides {
    /// Set via `anvil_setNextBlockPrevRandao`.
    pub(crate) prevrandao: NextBlockOverride<B256>,
    /// Set via `anvil_setNextBlockParentBeaconBlockRoot`.
    pub(crate) parent_beacon_block_root: NextBlockOverride<B256>,
}

/// A one-shot override of a value of the next mined block.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NextBlockOverride<T> {
    pub(crate) value: Option<T>,
    /// Bumped on every replacement, so a block consumes only the value it was built with.
    generation: u64,
}

impl<T> NextBlockOverride<T> {
    /// Replaces the value, returning the previous one.
    const fn replace(&mut self, value: Option<T>) -> Option<T> {
        self.generation = self.generation.wrapping_add(1);
        std::mem::replace(&mut self.value, value)
    }

    /// Clears the value if it is still the one `used` was taken from.
    fn consume(&mut self, used: &Self) {
        if self.generation == used.generation {
            self.value = None;
        }
    }
}

impl CheatEcrecover {
    pub const fn new(cheats: Arc<CheatsManager>) -> Self {
        Self { cheats }
    }
}

impl Precompile for CheatEcrecover {
    fn call(&self, input: PrecompileInput<'_>) -> PrecompileResult {
        if !self.cheats.has_recover_overrides() {
            return Ok(call_eth_precompile(
                ec_recover_run,
                input.data,
                input.gas(),
                input.reservoir,
            ));
        }

        const ECRECOVER_BASE: u64 = 3_000;
        if input.gas() < ECRECOVER_BASE {
            return Ok(PrecompileOutput::halt(PrecompileHalt::OutOfGas, input.reservoir));
        }
        let padded = right_pad::<128>(input.data);
        let v = padded[63];
        let mut sig_bytes = [0u8; 65];
        sig_bytes[..64].copy_from_slice(&padded[64..128]);
        sig_bytes[64] = v;
        let sig_bytes_wrapped = Bytes::from(sig_bytes);
        if let Some(addr) = self.cheats.get_recover_override(&sig_bytes_wrapped) {
            let mut out = [0u8; 32];
            out[12..].copy_from_slice(addr.as_slice());
            return Ok(PrecompileOutput::new(ECRECOVER_BASE, out.into(), input.reservoir));
        }
        Ok(call_eth_precompile(ec_recover_run, input.data, input.gas(), input.reservoir))
    }

    fn precompile_id(&self) -> &PrecompileId {
        &PRECOMPILE_ID_CHEAT_ECRECOVER
    }

    fn supports_caching(&self) -> bool {
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
    fn candidate_consumes_only_its_next_block_overrides() {
        let cheats = CheatsManager::default();
        let value = B256::with_last_byte(1);
        cheats.set_next_block_prevrandao(value);
        cheats.set_next_block_parent_beacon_block_root(value);
        let used = cheats.next_block_overrides();

        cheats.set_next_block_prevrandao(value);
        cheats.consume_next_block_overrides(&used);

        let overrides = cheats.next_block_overrides();
        assert_eq!(overrides.prevrandao.value, Some(value));
        assert_eq!(overrides.parent_beacon_block_root.value, None);
    }

    #[test]
    fn impersonate_returns_false_then_true() {
        let mgr = CheatsManager::default();
        let addr = Address::repeat_byte(1u8);
        assert!(!mgr.impersonate(addr));
        assert!(mgr.impersonate(addr));
    }
}
