use revm::{Env, SubRoutine};

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
