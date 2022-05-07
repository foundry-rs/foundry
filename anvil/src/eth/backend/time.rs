//! Manages the block time

use chrono::{DateTime, NaiveDateTime, Utc};
use parking_lot::RwLock;
use std::{sync::Arc, time::Duration};
use tracing::trace;

/// Returns the `Utc` datetime for the given seconds since unix epoch
pub fn utc_from_secs(secs: u64) -> DateTime<Utc> {
    DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(secs as i64, 0), Utc)
}

/// Manages block time
#[derive(Debug, Clone, Default)]
pub struct TimeManager {
    /// tracks the overall applied timestamp offset
    offset: Arc<RwLock<i128>>,
    /// Contains the next timestamp to use
    /// if this is set then the next time `[TimeManager::current_timestamp()]` is called this value
    /// will be taken and returned. After which the `offset` will be updated accordingly
    next_exact_timestamp: Arc<RwLock<Option<u64>>>,
}

// === impl TimeManager ===

impl TimeManager {
    fn offset(&self) -> i128 {
        *self.offset.read()
    }

    /// Adds the given `offset` to the already tracked offset
    fn add_offset(&self, offset: i128) {
        let mut current = self.offset.write();
        let next = current.saturating_add(offset);
        trace!(target: "time", "adding timestamp offset={}, total={}", offset, next);
        *current = next;
    }

    /// Jumps forward in time by the given seconds
    ///
    /// This will apply a permanent offset to the natural UNIX Epoch timestamp
    pub fn increase_time(&self, seconds: u64) {
        self.add_offset(seconds as i128)
    }

    /// Sets the exact timestamp to use in the next block
    pub fn set_next_block_timestamp(&self, timestamp: u64) {
        trace!(target: "time", "override next timestamp {}", timestamp);
        self.next_exact_timestamp.write().replace(timestamp);
    }

    /// Returns the current timestamp
    pub fn current_timestamp(&self) -> u64 {
        let current = duration_since_unix_epoch().as_secs() as i128;

        if let Some(next) = self.next_exact_timestamp.write().take() {
            // return the custom block timestamp and adjust the offset accordingly
            // the offset will be negative if the `next` timestamp is in the past
            let offset = (next as i128) - current;
            let mut current_offset = self.offset.write();
            *current_offset = offset;
            return next
        }

        current.saturating_add(self.offset()) as u64
    }
}

/// Returns the current duration since unix epoch.
pub fn duration_since_unix_epoch() -> Duration {
    use std::time::SystemTime;
    let now = SystemTime::now();
    now.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|err| panic!("Current time {:?} is invalid: {:?}", now, err))
}
