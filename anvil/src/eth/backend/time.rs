//! Manages the block time

use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct TimeManager {}

// === impl TimeManager ===

impl TimeManager {
    /// Returns the current timestamp
    pub fn current_timestamp(&self) -> u64 {
        duration_since_unix_epoch().as_secs()
    }
}

/// Returns the current duration since unix epoch.
pub fn duration_since_unix_epoch() -> Duration {
    use std::time::SystemTime;
    let now = SystemTime::now();
    now.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|err| panic!("Current time {:?} is invalid: {:?}", now, err))
}
