use crate::backend::JournaledState;
use alloy_evm::EvmEnv;
use alloy_primitives::{
    B256, U256,
    map::{AddressHashMap, U256Map},
};
use revm::state::AccountInfo;
use serde::{Deserialize, Serialize, Serializer, ser::SerializeStruct};
use std::collections::BTreeMap;

/// A minimal abstraction of a state at a certain point in time
#[derive(Clone, Debug, Default, Deserialize)]
pub struct StateSnapshot {
    pub accounts: AddressHashMap<AccountInfo>,
    pub storage: AddressHashMap<U256Map<U256>>,
    pub block_hashes: U256Map<B256>,
}

impl Serialize for StateSnapshot {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let accounts = self.accounts.iter().collect::<BTreeMap<_, _>>();
        let storage = self
            .storage
            .iter()
            .map(|(address, storage)| (address, storage.iter().collect::<BTreeMap<_, _>>()))
            .collect::<BTreeMap<_, _>>();
        let block_hashes = self.block_hashes.iter().collect::<BTreeMap<_, _>>();

        let mut state = serializer.serialize_struct("StateSnapshot", 3)?;
        state.serialize_field("accounts", &accounts)?;
        state.serialize_field("storage", &storage)?;
        state.serialize_field("block_hashes", &block_hashes)?;
        state.end()
    }
}

/// Represents a state snapshot taken during evm execution
#[derive(Clone, Debug)]
pub struct BackendStateSnapshot<T, SPEC, BLOCK> {
    pub db: T,
    /// Complete context state at a specific point.
    pub journaled_state: JournaledState,
    /// Contains the evm env at the time of the snapshot
    pub snap_evm_env: EvmEnv<SPEC, BLOCK>,
}

impl<T, SPEC, BLOCK> BackendStateSnapshot<T, SPEC, BLOCK> {
    /// Takes a new state snapshot.
    pub const fn new(db: T, journaled_state: JournaledState, evm_env: EvmEnv<SPEC, BLOCK>) -> Self {
        Self { db, journaled_state, snap_evm_env: evm_env }
    }

    /// Called when this state snapshot is reverted.
    ///
    /// Since we want to keep all additional logs that were emitted since the snapshot was taken
    /// we'll merge additional logs into the snapshot's `revm::JournaledState`. Additional logs are
    /// those logs that are missing in the snapshot's journaled_state, since the current
    /// journaled_state includes the same logs, we can simply replace use that See also
    /// `DatabaseExt::revert`.
    pub fn merge(&mut self, current: &JournaledState) {
        self.journaled_state.logs.clone_from(&current.logs);
    }
}

/// What to do when reverting a state snapshot.
///
/// Whether to remove the state snapshot or keep it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RevertStateSnapshotAction {
    /// Remove the state snapshot after reverting.
    #[default]
    RevertRemove,
    /// Keep the state snapshot after reverting.
    RevertKeep,
}

impl RevertStateSnapshotAction {
    /// Returns `true` if the action is to keep the state snapshot.
    pub const fn is_keep(&self) -> bool {
        matches!(self, Self::RevertKeep)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Address;

    #[test]
    fn state_snapshot_serializes_maps_in_key_order() {
        let low_address = Address::from_word(B256::from(U256::from(1)));
        let high_address = Address::from_word(B256::from(U256::from(2)));
        let low_slot = U256::from(1);
        let high_slot = U256::from(2);
        let mut snapshot = StateSnapshot::default();

        snapshot.accounts.insert(high_address, AccountInfo::from_balance(U256::from(2)));
        snapshot.accounts.insert(low_address, AccountInfo::from_balance(U256::from(1)));
        snapshot.storage.insert(
            high_address,
            [(high_slot, U256::from(200)), (low_slot, U256::from(100))].into_iter().collect(),
        );
        snapshot.storage.insert(
            low_address,
            [(high_slot, U256::from(200)), (low_slot, U256::from(100))].into_iter().collect(),
        );
        snapshot.block_hashes.insert(high_slot, B256::from(U256::from(200)));
        snapshot.block_hashes.insert(low_slot, B256::from(U256::from(100)));

        let json = serde_json::to_string(&snapshot).unwrap();
        let storage_start = json.find("\"storage\"").unwrap();
        let block_hashes_start = json.find("\"block_hashes\"").unwrap();
        let accounts = &json[..storage_start];
        let storage = &json[storage_start..block_hashes_start];
        let block_hashes = &json[block_hashes_start..];
        let low_address = serde_json::to_string(&low_address).unwrap();
        let high_address = serde_json::to_string(&high_address).unwrap();
        let low_slot = serde_json::to_string(&low_slot).unwrap();
        let high_slot = serde_json::to_string(&high_slot).unwrap();

        assert!(accounts.find(&low_address).unwrap() < accounts.find(&high_address).unwrap());
        let low_storage_start = storage.find(&low_address).unwrap();
        let high_storage_start = storage.find(&high_address).unwrap();
        assert!(low_storage_start < high_storage_start);
        let low_storage = &storage[low_storage_start..high_storage_start];
        let high_storage = &storage[high_storage_start..];
        assert!(low_storage.find(&low_slot).unwrap() < low_storage.find(&high_slot).unwrap());
        assert!(high_storage.find(&low_slot).unwrap() < high_storage.find(&high_slot).unwrap());
        assert!(block_hashes.find(&low_slot).unwrap() < block_hashes.find(&high_slot).unwrap());
    }
}
