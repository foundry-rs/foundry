//! User-visible state snapshots.

use super::{EvmEnv, ForkRegistry, SendableExt, StoredExt};
use evm2::{
    EvmFeatures, EvmTypesHost,
    evm::{EmptyDB, State, StateCheckpoint, StateSnapshot},
};
use std::{fmt, mem};

/// Everything a snapshot revert restores.
///
/// evm2's [`StateSnapshot`] covers the running context's accepted and in-flight state but not its
/// backing database; the fork registry supplies that, along with every inactive fork's accepted
/// state. Cheatcode state rides along as `C`.
pub struct LifecycleSnapshot<T: EvmTypesHost, C> {
    /// The running context's state.
    pub(super) state: StateSnapshot,
    /// Forks, persistent accounts and the active selection.
    pub(super) forks: ForkRegistry<T>,
    /// Environment of the running context.
    pub(super) env: EvmEnv<T>,
    /// Network extension state of the running context.
    pub(super) ext: StoredExt<T::EvmExt>,
    /// Caller-owned state, such as cheatcode state.
    pub(super) extra: C,
}

impl<T: EvmTypesHost<EvmExt: SendableExt>, C: Clone> Clone for LifecycleSnapshot<T, C> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            forks: self.forks.clone(),
            env: self.env,
            ext: self.ext.clone(),
            extra: self.extra.clone(),
        }
    }
}

impl<T: EvmTypesHost, C> fmt::Debug for LifecycleSnapshot<T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LifecycleSnapshot").field("forks", &self.forks).finish_non_exhaustive()
    }
}

/// A running state detached from its backing database, such as the one a snapshot restoration
/// replaced.
#[derive(Clone, Debug)]
pub struct DetachedState(State<'static>);

// SAFETY: The backing database is the only part of `State` that is not `Send` and `Sync`, and a
// detached state's is an `EmptyDB`.
unsafe impl Send for DetachedState {}
unsafe impl Sync for DetachedState {}

impl DetachedState {
    /// Detaches `state` from its backing database.
    ///
    /// Materialize a state that reads accepted state through a shared view first; that view is
    /// its backing database.
    pub fn new(mut state: State<'_>) -> Self {
        state.overlay_db_mut().db = Box::new(EmptyDB::default());
        // SAFETY: `State`'s lifetime only bounds its backing database, which was just replaced by
        // an owned one.
        Self(unsafe { mem::transmute::<State<'_>, State<'static>>(state) })
    }
}

/// Running states replaced by snapshot restorations inside the open call frames of a transaction.
///
/// A restoration replaces the running state, journal included, so evm2 can no longer unwind it
/// with the checkpoint of a frame that was open when it happened. Those frames own it instead:
/// when one of them fails, the state the restoration replaced is reinstated and rolled back to the
/// failing frame's checkpoint, which undoes the restoration together with the frame's other
/// writes, as a reverted or halted frame does. A frame that succeeds keeps the restoration.
///
/// The backend, active fork and environment a restoration installed are kept either way, and the
/// reinstated state reads from the backing database of the running state it replaces.
///
/// Only the first restoration since a frame's caller started is retained, which is the one that
/// caller needs to unwind, so retained states are bounded by call depth. A nested transaction has
/// its own journal and needs its own instance.
#[derive(Clone, Debug, Default)]
pub struct SnapshotRestores {
    frames: Vec<RestoreFrame>,
    replaced: Vec<DetachedState>,
    /// Whether a restoration survived the outermost frame, whose retained states are released.
    kept: bool,
}

#[derive(Clone, Debug)]
struct RestoreFrame {
    /// Cursors evm2 rolls the frame back to if it fails, once evm2 runs it.
    checkpoint: Option<StateCheckpoint>,
    /// Log count of a create frame whose checkpoint is completed when its init code starts.
    pending_create: Option<usize>,
    /// Number of retained states when the frame started.
    replaced_len: usize,
}

impl SnapshotRestores {
    /// Opens a frame. A call frame evm2 runs then gets its checkpoint from
    /// [`Self::set_call_checkpoint`]; a frame without one cannot undo restorations.
    pub fn frame_start(&mut self) {
        self.frames.push(RestoreFrame {
            checkpoint: None,
            pending_create: None,
            replaced_len: self.replaced.len(),
        });
    }

    /// Records the checkpoint of the innermost call frame, which evm2 takes after the call hooks
    /// when it runs the frame.
    pub fn set_call_checkpoint(&mut self, state: &State<'_>) {
        if let Some(frame) = self.frames.last_mut() {
            frame.checkpoint = Some(state.checkpoint());
        }
    }

    /// Opens a create frame after its hooks, when no log is emitted before evm2's checkpoint.
    pub fn create_start(&mut self, state: &State<'_>) {
        self.frames.push(RestoreFrame {
            checkpoint: None,
            pending_create: Some(state.logs().len()),
            replaced_len: self.replaced.len(),
        });
    }

    /// Records the checkpoint of the innermost create frame when its init code starts.
    ///
    /// evm2 takes it after bumping the creator's nonce, then journals the endowment, if any, and
    /// the new account.
    pub fn init_code_start(&mut self, state: &State<'_>, endowment: bool) {
        if let Some(frame) = self.frames.last_mut()
            && let Some(logs_len) = frame.pending_create.take()
        {
            let created = 1 + usize::from(endowment);
            let journal_len = state.journal().len().saturating_sub(created);
            frame.checkpoint = Some(StateCheckpoint::new(journal_len, logs_len));
        }
    }

    /// Retains `replaced`, the running state a restoration in the innermost frame (the cheatcode
    /// call) replaced, if it is the first restoration since that frame's caller started.
    pub fn capture(&mut self, replaced: DetachedState) {
        if let Some(caller) = self.frames.len().checked_sub(2).map(|i| &self.frames[i])
            && caller.replaced_len == self.replaced.len()
        {
            self.replaced.push(replaced);
        }
    }

    /// Closes the innermost frame. If it failed, reinstates the state replaced by the first
    /// restoration made inside it, rolled back to the frame's checkpoint.
    pub fn frame_end(&mut self, state: &mut State<'_>, failed: bool, features: EvmFeatures) {
        let Some(frame) = self.frames.pop() else { return };
        if failed && let Some(checkpoint) = frame.checkpoint {
            if self.replaced.len() > frame.replaced_len {
                self.replaced.truncate(frame.replaced_len + 1);
                let DetachedState(replaced) =
                    self.replaced.pop().expect("a restoration was retained");
                let restored = mem::replace(state, replaced);
                state.overlay_db_mut().db = restored.into_db();
            }
            // evm2 already rolled back to this checkpoint unless the journal was replaced since.
            state.rollback(checkpoint, features);
        }
        // The caller only needs the first restoration made since it started.
        if let Some(caller) = self.frames.last() {
            self.replaced.truncate(caller.replaced_len + 1);
        } else {
            self.kept |= !self.replaced.is_empty();
            self.replaced.clear();
        }
    }

    /// Moves the checkpoints of the open frames to the end of the running journal after a context
    /// switch replaced it, such as a fork selection, and drops the retained states.
    ///
    /// The switch accepted the writes and restorations made before it, so a failing frame only
    /// undoes the writes made since.
    pub fn journal_replaced(&mut self, state: &State<'_>) {
        let journal_len = state.journal().len();
        for frame in &mut self.frames {
            frame.replaced_len = 0;
            if let Some(checkpoint) = &mut frame.checkpoint {
                *checkpoint = StateCheckpoint::new(journal_len, checkpoint.logs_len());
            }
        }
        self.replaced.clear();
    }

    /// Returns the number of open frames.
    pub const fn depth(&self) -> usize {
        self.frames.len()
    }

    /// Closes the frames opened above `depth` without undoing restorations, for frames that
    /// aborted with an execution error and never ended.
    pub fn abandon(&mut self, depth: usize) {
        self.frames.truncate(depth);
    }

    /// Returns whether a restoration survived the outermost frame.
    pub const fn kept(&self) -> bool {
        self.kept
    }
}
