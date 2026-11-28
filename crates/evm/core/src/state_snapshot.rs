//! Support for snapshotting different states

use alloy_primitives::{U256, map::HashMap};

/// Represents all state snapshots
#[derive(Clone, Debug)]
pub struct StateSnapshots<T> {
    id: U256,
    state_snapshots: HashMap<U256, T>,
}

impl<T> StateSnapshots<T> {
    const fn next_id(&mut self) -> U256 {
        let id = self.id;
        self.id = id.saturating_add(U256::ONE);
        id
    }

    /// Returns the state snapshot with the given id `id`
    pub fn get(&self, id: U256) -> Option<&T> {
        self.state_snapshots.get(&id)
    }

    /// Returns `true` if there are no state snapshots.
    pub fn is_empty(&self) -> bool {
        self.state_snapshots.is_empty()
    }

    /// Removes the state snapshot with the given `id`.
    ///
    /// If the snapshot exists, also removes all snapshots taken after it.
    /// If it is absent, returns `None` and preserves the remaining snapshots.
    pub fn remove(&mut self, id: U256) -> Option<T> {
        let snapshot_state = self.state_snapshots.remove(&id)?;

        // Revert all state snapshots taken after the state snapshot with the `id`
        let mut to_revert = id + U256::ONE;
        while to_revert < self.id {
            self.state_snapshots.remove(&to_revert);
            to_revert += U256::ONE;
        }

        Some(snapshot_state)
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
    pub fn insert_at(&mut self, state_snapshot: T, id: U256) {
        self.state_snapshots.insert(id, state_snapshot);
    }
}

impl<T> Default for StateSnapshots<T> {
    fn default() -> Self {
        Self { id: U256::ZERO, state_snapshots: HashMap::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverting_deleted_snapshot_preserves_newer_snapshot() {
        let mut snapshots = StateSnapshots::default();
        let old = snapshots.insert("old");
        let new = snapshots.insert("new");
        assert_eq!(snapshots.remove_at(old), Some("old"));
        assert_eq!(snapshots.remove(old), None);
        assert_eq!(snapshots.get(new), Some(&"new"));
    }

    #[derive(Clone, Debug, ephyra::Arbitrary)]
    enum Op {
        Save,
        Delete(u8),
        Revert(u8),
    }

    #[ephyra::test]
    fn snapshot_operations_match_model(ops: Vec<Op>) {
        let mut snapshots = StateSnapshots::default();
        let mut model = Vec::new();
        for op in ops {
            match op {
                Op::Save => {
                    let id = model.len();
                    assert_eq!(snapshots.insert(id), U256::from(id));
                    model.push(Some(id));
                }
                Op::Delete(index) | Op::Revert(index) => {
                    let index = usize::from(index) % (model.len() + 1);
                    let expected = model.get_mut(index).and_then(Option::take);
                    let id = U256::from(index);
                    if matches!(op, Op::Revert(_)) {
                        assert_eq!(snapshots.remove(id), expected);
                        if expected.is_some() {
                            model[index..].fill(None);
                        }
                    } else {
                        assert_eq!(snapshots.remove_at(id), expected);
                    }
                }
            }
            for (index, value) in model.iter().enumerate() {
                assert_eq!(snapshots.get(U256::from(index)), value.as_ref());
            }
        }
    }
}
