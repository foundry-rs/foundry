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
/// The failing frame also keeps the persistent writes made since the restoration, which it
/// replays over the reinstated state after the writes its rollback undid. They were made on the
/// state the restoration installed, so the ones that set a balance or nonce, such as `deal`,
/// replay as the values they left, while changes such as a nested transaction's value transfers
/// keep replaying as deltas, see [`Replay`](super::Replay). The writes later restorations in the
/// frame dropped from the running ledger are kept too, since the failing frame also undoes those
/// restorations. Once a context switch, which the failing frame keeps, drops them from the running
/// ledger, none of them are kept.
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
    /// Each retained state.
    replaced: Vec<ReplacedState>,
    /// Whether a restoration survived the outermost frame.
    kept: bool,
}

/// A running state a restoration replaced.
#[derive(Clone, Debug)]
struct ReplacedState {
    state: StateSnapshot,
    /// The persistent writes recorded when the state was replaced.
    ledger: Ledger,
    /// The persistent writes recorded since, or `None` once a context switch dropped them.
    later: Option<LaterWrites>,
    /// The length of `dropped` and the `since` of the `later` writes of each earlier retained
    /// state when this one was replaced, which reinstating this state restores along with its
    /// ledger.
    earlier: Vec<Option<(usize, usize)>>,
}

/// The persistent writes recorded since a restoration.
#[derive(Clone, Debug)]
struct LaterWrites {
    /// The ones later restorations dropped from the running ledger.
    dropped: Ledger,
    /// The position in the running ledger past which it holds the rest.
    since: usize,
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
    /// frame started. The restoration keeps the first `len` writes of `ledger`.
    pub fn capture(
        &mut self,
        replaced: StateSnapshot,
        ledger: &[(usize, usize, PersistentWrite)],
        len: usize,
    ) {
        let len = len.min(ledger.len());
        let first =
            self.frames.last().is_some_and(|frame| frame.replaced_len == self.replaced.len());
        let earlier = if first {
            let lengths = |later: &LaterWrites| (later.dropped.len(), later.since);
            self.replaced.iter().map(|retained| retained.later.as_ref().map(lengths)).collect()
        } else {
            Vec::new()
        };
        // The running ledger only keeps its first `len` writes, so each retained restoration sets
        // aside the ones recorded since it and continues past `len`.
        for later in self.replaced.iter_mut().filter_map(|retained| retained.later.as_mut()) {
            later.dropped.extend(ledger.iter().skip(later.since).cloned());
            later.since = len;
        }
        if first {
            self.replaced.push(ReplacedState {
                state: replaced,
                ledger: ledger.to_vec(),
                later: Some(LaterWrites { dropped: Vec::new(), since: len }),
                earlier,
            });
        }
    }

    /// Stops keeping the persistent writes recorded since each retained restoration, after a
    /// context switch accepted the running state and dropped them. Failing frames keep the switch,
    /// so reinstating a state does not keep them again.
    pub fn accept_writes(&mut self) {
        for retained in &mut self.replaced {
            retained.later = None;
            retained.earlier.fill(None);
        }
    }

    /// Closes the interpreter frame at `depth`, if one is open. If it failed, reinstates the state
    /// replaced by the first restoration made inside it, rolled back to the frame's checkpoint,
    /// and replaces `ledger` with the persistent writes to replay over it.
    ///
    /// Returns the checkpoint of the closed frame.
    pub fn frame_end(
        &mut self,
        state: &mut State<'_>,
        ledger: &mut Ledger,
        depth: u16,
        failed: bool,
        features: EvmFeatures,
    ) -> Option<StateCheckpoint> {
        // Frames above `depth` aborted with an execution error and never ended.
        while self.frames.pop_if(|frame| frame.depth > depth).is_some() {}
        let frame = self.frames.pop_if(|frame| frame.depth == depth)?;
        if failed && self.replaced.len() > frame.replaced_len {
            self.replaced.truncate(frame.replaced_len + 1);
            let replaced = self.replaced.pop().expect("a restoration was retained");
            let db = mem::replace(&mut state.overlay_db_mut().db, Box::new(EmptyDB::default()));
            *state = replaced.state.into_state(EmptyDB::default());
            state.overlay_db_mut().db = db;
            state.rollback(frame.checkpoint.clone(), features);
            // The writes recorded since the restoration were made on the state it installed, so
            // the frame's rollback did not reach them, and the ones that set a balance or nonce
            // set the value they left. Marking them past its checkpoint has them replayed after
            // the writes recorded over the reinstated state.
            let running = mem::replace(ledger, replaced.ledger);
            if let Some(later) = replaced.later {
                let writes = later.dropped.into_iter().chain(running.into_iter().skip(later.since));
                ledger.extend(
                    writes.map(|(depth, _, write)| (depth, usize::MAX, write.reinstated())),
                );
            }
            // The reinstated ledger holds the writes the restoration and the later ones dropped,
            // so earlier restorations keep them in the running ledger again.
            for (retained, earlier) in self.replaced.iter_mut().zip(replaced.earlier) {
                retained.later =
                    retained.later.take().zip(earlier).map(|(mut later, (dropped, since))| {
                        later.dropped.truncate(dropped);
                        later.since = since;
                        later
                    });
            }
        }
        // The caller only needs the first restoration made since it started.
        if let Some(caller) = self.frames.last() {
            self.replaced.truncate(caller.replaced_len + 1);
        } else {
            self.kept |= !self.replaced.is_empty();
            self.replaced.clear();
        }
        Some(frame.checkpoint)
    }

    /// Returns whether a restoration survived the outermost frame.
    pub const fn kept(&self) -> bool {
        self.kept
    }
}
