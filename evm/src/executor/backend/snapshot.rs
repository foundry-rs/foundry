use ethers::types::{Address, H256, U256};
use hashbrown::HashMap as Map;
use revm::{AccountInfo, Env, SubRoutine};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A minimal abstraction of a state at a certain point in time
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub accounts: BTreeMap<Address, AccountInfo>,
    pub storage: BTreeMap<Address, BTreeMap<U256, U256>>,
    pub block_hashes: Map<U256, H256>,
}

/// Represents a snapshot taken during evm execution
#[derive(Clone, Debug)]
pub struct BackendSnapshot<T> {
    pub db: T,
    /// The subroutine state at a specific point
    pub subroutine: SubRoutine,
    /// Contains the env at the time of the snapshot
    pub env: Env,
}

// === impl BackendSnapshot ===

impl<T> BackendSnapshot<T> {
    /// Takes a new snapshot
    pub fn new(db: T, subroutine: SubRoutine, env: Env) -> Self {
        Self { db, subroutine, env }
    }

    /// Called when this snapshot is reverted.
    ///
    /// Since we want to keep all additional logs that were emitted since the snapshot was taken
    /// we'll merge additional logs into the snapshot's `revm::Subroutine`. Additional logs are
    /// those logs that are missing in the snapshot's subroutine, since the current subroutine
    /// includes the same logs, we can simply replace use that See also `DatabaseExt::revert`
    pub fn merge(&mut self, current: &SubRoutine) {
        self.subroutine.logs = current.logs.clone();
    }
}
