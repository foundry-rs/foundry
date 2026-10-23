//! Debug steps recorded straight into a SolDB step table.

use soldb_core::StepTable;
use std::{
    fmt,
    ops::Range,
    sync::{Arc, Mutex, MutexGuard},
};

/// The step table that the debug recorders of one session append to, in execution order.
///
/// Each executor call runs its own clone of the inspector stack. The clones share this sink, so
/// the steps of the deployment, the setup, and the test go into one table.
#[derive(Clone)]
pub struct DebugStepSink(Arc<Mutex<Option<StepTable>>>);

impl DebugStepSink {
    /// An empty sink.
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(Some(StepTable::new()))))
    }

    /// Takes the table, or returns `None` if a recorder holds it.
    pub fn take(&self) -> Option<StepTable> {
        self.lock().take()
    }

    /// Gives the table back.
    pub fn put(&self, table: StepTable) {
        *self.lock() = Some(table);
    }

    /// Locks the table. It is `None` while a recorder holds it.
    pub fn lock(&self) -> MutexGuard<'_, Option<StepTable>> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether `self` and `other` are the same sink.
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Default for DebugStepSink {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for DebugStepSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DebugStepSink").finish_non_exhaustive()
    }
}

/// The steps that one inspector stack recorded for its call trace arena.
#[derive(Clone, Debug)]
pub struct DebugRecording {
    /// The table that holds the steps.
    pub sink: DebugStepSink,
    /// The steps of the arena in the sink's table.
    pub steps: Range<usize>,
    /// The steps of each arena node, by node index. The step indices count from `steps.start`.
    pub frames: Vec<RecordedFrame>,
}

/// The steps that one call or creation spans, its subcalls included.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecordedFrame {
    /// The journal depth of the call, as the call trace records it.
    pub depth: usize,
    /// The first step of the frame.
    pub entry_step: usize,
    /// The step after the last step of the frame.
    pub exit_step: usize,
}
