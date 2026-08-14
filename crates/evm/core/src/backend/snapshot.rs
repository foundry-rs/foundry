use crate::FoundryContextState;
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
pub struct BackendStateSnapshot<T, SPEC, BLOCK, AUX> {
    pub db: T,
    /// Complete context state at a specific point.
    pub context_state: FoundryContextState<AUX>,
    /// Contains the evm env at the time of the snapshot
    pub snap_evm_env: EvmEnv<SPEC, BLOCK>,
}

impl<T, SPEC, BLOCK, AUX> BackendStateSnapshot<T, SPEC, BLOCK, AUX> {
    /// Takes a new state snapshot.
    pub const fn new(
        db: T,
        context_state: FoundryContextState<AUX>,
        evm_env: EvmEnv<SPEC, BLOCK>,
    ) -> Self {
        Self { db, context_state, snap_evm_env: evm_env }
    }

    /// Called when this state snapshot is reverted.
    ///
    /// Since we want to keep all additional logs that were emitted since the snapshot was taken
    /// we'll merge additional logs into the snapshot's `revm::JournaledState`. Additional logs are
    /// those logs that are missing in the snapshot's journaled_state, since the current
    /// journaled_state includes the same logs, we can simply replace use that See also
    /// `DatabaseExt::revert`.
    pub fn merge(&mut self, current: &FoundryContextState<AUX>) {
        self.context_state.journaled_state.logs.clone_from(&current.journaled_state.logs);
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
