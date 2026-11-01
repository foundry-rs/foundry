//! User-visible state snapshots.

use super::{EvmEnv, ForkRegistry, PersistentWrite, SendableExt, StoredExt};
use evm2::{
    EvmFeatures, EvmTypesHost,
    evm::{EmptyDB, State, StateCheckpoint, StateSnapshot},
};
use std::{fmt, mem};

/// Persistent writes with the depth and journal length they were recorded at.
type Ledger = Vec<(usize, usize, PersistentWrite)>;

/// Everything a snapshot revert restores.
///
/// evm2's [`StateSnapshot`] covers the running context's accepted and in-flight state but not its
/// backing database; the fork registry supplies that, along with every inactive fork's accepted
/// state.
pub struct LifecycleSnapshot<T: EvmTypesHost> {
    /// The running context's state.
    pub(super) state: StateSnapshot,
    /// Forks, persistent accounts and the active selection.
    pub(super) forks: ForkRegistry<T>,
    /// Environment of the running context.
    pub(super) env: EvmEnv<T>,
    /// Network extension state of the running context.
    pub(super) ext: StoredExt<T::EvmExt>,
}

impl<T: EvmTypesHost<EvmExt: SendableExt>> Clone for LifecycleSnapshot<T> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            forks: self.forks.clone(),
            env: self.env,
            ext: self.ext.clone(),
        }
    }
}

impl<T: EvmTypesHost> fmt::Debug for LifecycleSnapshot<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LifecycleSnapshot").field("forks", &self.forks).finish_non_exhaustive()
    }
}

/// Running states replaced by snapshot restorations inside the open frames of a transaction.
///
/// A restoration replaces the running state, journal included, so evm2 can no longer unwind it
/// with the checkpoint of a frame that was open when it happened. Those frames own it instead:
/// when one of them fails, the state the restoration replaced is reinstated and rolled back to the
/// failing frame's checkpoint, which undoes the restoration together with the frame's earlier
/// writes, as a reverted or halted frame does. A frame that succeeds keeps the restoration, and
/// its caller takes over the retained state.
///
/// The backend, active fork and environment a restoration installed are kept either way, and the
/// reinstated state reads from the backing database of the running state it replaces. Retained
/// states and checkpoints also survive context switches that replace the running journal, such as
/// fork selections, so a frame that fails after selecting another fork still reinstates the state
/// a restoration replaced on the source fork.
///
/// Only the first restoration since a frame started is retained, which is the one that frame needs
/// to unwind, so retained states are bounded by call depth. A nested transaction has its own
/// journal and needs its own instance.
#[derive(Clone, Debug, Default)]
pub struct SnapshotRestores {
    frames: Vec<RestoreFrame>,
    /// Each retained state, with the persistent writes recorded when it was replaced.
    replaced: Vec<(StateSnapshot, Ledger)>,
    /// Whether a restoration survived the outermost frame.
    kept: bool,
}

#[derive(Clone, Debug)]
struct RestoreFrame {
    depth: u16,
    /// The checkpoint evm2 rolls the frame back to if it fails.
    checkpoint: StateCheckpoint,
    /// Number of retained states when the frame started.
    replaced_len: usize,
}

impl SnapshotRestores {
    /// Opens the interpreter frame at `depth`, which rolls back to `checkpoint` if it fails.
    pub fn frame_start(&mut self, depth: u16, checkpoint: StateCheckpoint) {
        self.frames.push(RestoreFrame { depth, checkpoint, replaced_len: self.replaced.len() });
    }

    /// Retains `replaced`, the running state a restoration in the innermost frame replaced, with
    /// the persistent writes `ledger` recorded over it, if it is the first restoration since that
    /// frame started.
    pub fn capture(&mut self, replaced: StateSnapshot, ledger: &[(usize, usize, PersistentWrite)]) {
        if self.frames.last().is_some_and(|frame| frame.replaced_len == self.replaced.len()) {
            self.replaced.push((replaced, ledger.to_vec()));
        }
    }

    /// Closes the interpreter frame at `depth`, if one is open. If it failed, reinstates the state
    /// replaced by the first restoration made inside it, rolled back to the frame's checkpoint.
    ///
    /// Returns the checkpoint of the closed frame, and the persistent writes recorded over a
    /// reinstated state.
    pub fn frame_end(
        &mut self,
        state: &mut State<'_>,
        depth: u16,
        failed: bool,
        features: EvmFeatures,
    ) -> Option<(StateCheckpoint, Option<Ledger>)> {
        // Frames above `depth` aborted with an execution error and never ended.
        while self.frames.pop_if(|frame| frame.depth > depth).is_some() {}
        let frame = self.frames.pop_if(|frame| frame.depth == depth)?;
        let mut ledger = None;
        if failed && self.replaced.len() > frame.replaced_len {
            self.replaced.truncate(frame.replaced_len + 1);
            let (replaced, writes) = self.replaced.pop().expect("a restoration was retained");
            ledger = Some(writes);
            let db = mem::replace(&mut state.overlay_db_mut().db, Box::new(EmptyDB::default()));
            *state = replaced.into_state(EmptyDB::default());
            state.overlay_db_mut().db = db;
            state.rollback(frame.checkpoint.clone(), features);
        }
        // The caller only needs the first restoration made since it started.
        if let Some(caller) = self.frames.last() {
            self.replaced.truncate(caller.replaced_len + 1);
        } else {
            self.kept |= !self.replaced.is_empty();
            self.replaced.clear();
        }
        Some((frame.checkpoint, ledger))
    }

    /// Returns whether a restoration survived the outermost frame.
    pub const fn kept(&self) -> bool {
        self.kept
    }
}
