use proptest::bits::{BitSetLike, VarBitSet};

#[derive(Clone, Copy, Debug)]
struct Shrink {
    call_index: usize,
}

/// Shrinker for a call sequence failure.
/// Iterates sequence call sequence top down and removes calls one by one.
/// If the failure is still reproducible with removed call then moves to the next one.
/// If the failure is not reproducible then restore removed call and moves to next one.
#[derive(Debug)]
pub(crate) struct CallSequenceShrinker {
    /// Call sequence to be shrinked.
    call_sequence: Vec<usize>,
    /// Call ids contained in current shrinked sequence.
    included_calls: VarBitSet,
    /// Current shrinked call id.
    shrink: Shrink,
    /// Previous shrinked call id.
    prev_shrink: Option<Shrink>,
}

impl CallSequenceShrinker {
    pub(crate) fn new(call_sequence: Vec<usize>) -> Self {
        let included_calls = VarBitSet::saturated(call_sequence.len());
        Self { call_sequence, included_calls, shrink: Shrink { call_index: 0 }, prev_shrink: None }
    }

    /// Return candidate shrink sequence to be tested, by removing ids from original sequence.
    pub(crate) fn current(&self) -> Vec<usize> {
        self.call_sequence
            .iter()
            .enumerate()
            .filter(|&(call_id, _)| self.included_calls.test(call_id))
            .map(|(_, element)| element.to_owned())
            .collect()
    }

    /// Removes next call from sequence.
    pub(crate) fn simplify(&mut self) -> bool {
        if self.shrink.call_index >= self.call_sequence.len() {
            // We reached the end of call sequence, nothing left to simplify.
            false
        } else {
            // Remove current call.
            self.included_calls.clear(self.shrink.call_index);
            // Record current call as previous call.
            self.prev_shrink = Some(self.shrink);
            // Remove next call index
            self.shrink = Shrink { call_index: self.shrink.call_index + 1 };
            true
        }
    }

    /// Reverts removed call from sequence.
    pub(crate) fn complicate(&mut self) -> bool {
        match self.prev_shrink {
            Some(shrink) => {
                // Undo the last call removed.
                self.included_calls.set(shrink.call_index);
                self.prev_shrink = None;
                true
            }
            None => false,
        }
    }
}
