//! Support for snapshotting different states

use alloy_primitives::U256;
use std::{collections::HashMap, ops::Add};

/// Represents all state snapshots
#[derive(Clone, Debug)]
pub struct StateSnapshots<T> {
    id: U256,
    state_snapshots: HashMap<U256, T>,
}

impl<T> StateSnapshots<T> {
    fn next_id(&mut self) -> U256 {
        let id = self.id;
        self.id = id.saturating_add(U256::from(1));
        id
    }

    /// Returns the state snapshot with the given id `id`
    pub fn get(&self, id: U256) -> Option<&T> {
        self.state_snapshots.get(&id)
    }

    /// Removes the state snapshot with the given `id`.
    ///
    /// This will also remove any state snapshots taken after the state snapshot with the `id`.
    /// e.g.: reverting to id 1 will delete snapshots with ids 1, 2, 3, etc.)
    pub fn remove(&mut self, id: U256) -> Option<T> {
        let snapshot_state = self.state_snapshots.remove(&id);

        // Revert all state snapshots taken after the state snapshot with the `id`
        let mut to_revert = id.add(U256::from(1));
        while to_revert < self.id {
            self.state_snapshots.remove(&to_revert);
            to_revert += U256::from(1);
        }

        snapshot_state
    }

    /// Removes all state snapshots.
    pub fn clear(&mut self) {
        self.state_snapshots.clear();
    }

    /// Removes the state snapshot with the given `id`.
    ///
    /// Does not remove state snapshots after it.
    pub fn remove_at(&mut self, id: U256) -> Option<T> {
        self.state_snapshots.remove(&id)
    }

    /// Inserts the new state snapshot and returns the id.
    pub fn insert(&mut self, state_snapshot: T) -> U256 {
        let id = self.next_id();
        self.state_snapshots.insert(id, state_snapshot);
        id
    }

    /// Inserts the new state snapshot at the given `id`.
    ///
    ///  Does not auto-increment the next `id`.
    pub fn insert_at(&mut self, state_snapshot: T, id: U256) -> U256 {
        self.state_snapshots.insert(id, state_snapshot);
        id
    }
}

impl<T> Default for StateSnapshots<T> {
    fn default() -> Self {
        Self { id: U256::ZERO, state_snapshots: HashMap::new() }
    }
}
