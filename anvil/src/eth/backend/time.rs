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
    /// The interval to use when determining the next block's timestamp
    interval: Arc<RwLock<Option<TimestampInterval>>>,
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

    /// Sets the timestamp we should base further timestamps on
    pub fn set_start_timestamp(&self, seconds: u64) {
        let current = duration_since_unix_epoch().as_secs() as i128;
        *self.offset.write() = (seconds as i128) - current;
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

    /// Sets an interval to use when computing the next timestamp
    ///
    /// If an interval already exists, this will update the interval, otherwise a new interval will be set starting with the current timestamp
    pub fn set_block_timestamp_interval(&self, interval: u64) {
        trace!(target: "time", "set interval {}", interval);
        let mut current = self.interval.write();
        if let Some(current) = current.as_mut() {
            current.interval = interval;
        } else {
            *current =
                Some(TimestampInterval { interval, last_timestamp: self.current_call_timestamp() });
        }
    }

    /// Returns the current timestamp and updates the underlying offset accordingly
    pub fn next_timestamp(&self) -> u64 {
        let current = duration_since_unix_epoch().as_secs() as i128;

        if let Some(next) = self.next_exact_timestamp.write().take() {
            // return the custom block timestamp and adjust the offset accordingly
            // the offset will be negative if the `next` timestamp is in the past
            let offset = (next as i128) - current;
            let mut current_offset = self.offset.write();
            // increase the offset by one second, so that we don't yield the same timestamp twice if
            // it's set manually
            *current_offset = offset.saturating_add(1);
            return next
        }

        current.saturating_add(self.offset()) as u64
    }

    /// Returns the current timestamp for a call that does not update the value
    pub fn current_call_timestamp(&self) -> u64 {
        let mut current = duration_since_unix_epoch().as_secs() as i128;

        if let Some(next) = *self.next_exact_timestamp.read() {
            // return the custom block timestamp and adjust the offset accordingly
            // the offset will be negative if the `next` timestamp is in the past
            let offset = (next as i128) - current;
            current = current.saturating_add(offset)
        }

        current as u64
    }
}

/// Provides a tick rate for the time manager
///
/// While the timestamp is based on the unix epoch, it
#[derive(Debug, Clone)]
struct TimestampInterval {
    interval: u64,
    last_timestamp: u64,
}

/// Returns the current duration since unix epoch.
pub fn duration_since_unix_epoch() -> Duration {
    use std::time::SystemTime;
    let now = SystemTime::now();
    now.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|err| panic!("Current time {:?} is invalid: {:?}", now, err))
}
