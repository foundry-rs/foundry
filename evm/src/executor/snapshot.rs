//! support for snapshotting different states

use ethers::types::U256;
use std::collections::HashMap;

/// Represents all snapshots
#[derive(Debug, Clone)]
pub struct Snapshots<T> {
    id: U256,
    snapshots: HashMap<U256, T>,
}

// === impl Snapshots ===

impl<T> Snapshots<T> {
    fn next_id(&mut self) -> U256 {
        let id = self.id;
        self.id = id.saturating_add(U256::one());
        id
    }

    /// Returns the snapshot with the given id `id`
    pub fn get(&self, id: U256) -> Option<&T> {
        self.snapshots.get(&id)
    }

    /// Removes the snapshot with the given `id`.
    ///
    /// This will also remove any snapshots taken after the snapshot with the `id`. e.g.: reverting
    /// to id 1 will delete snapshots with ids 1, 2, 3, etc.)
    pub fn remove(&mut self, id: U256) -> Option<T> {
        let snapshot = self.snapshots.remove(&id);

        // revert all snapshots taken after the snapshot
        let mut to_revert = id + 1;
        while to_revert < self.id {
            self.snapshots.remove(&to_revert);
            to_revert = to_revert + 1;
        }

        snapshot
    }

    /// Inserts the new snapshot and returns the id
    pub fn insert(&mut self, snapshot: T) -> U256 {
        let id = self.next_id();
        self.snapshots.insert(id, snapshot);
        id
    }
}

impl<T> Default for Snapshots<T> {
    fn default() -> Self {
        Self { id: U256::zero(), snapshots: HashMap::new() }
    }
}
