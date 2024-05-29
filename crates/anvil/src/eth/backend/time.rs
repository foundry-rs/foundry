//! Manages the block time

use crate::eth::error::BlockchainError;
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use std::{sync::Arc, time::Duration};

/// Returns the `Utc` datetime for the given seconds since unix epoch
pub fn utc_from_secs(secs: u64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs as i64, 0).unwrap()
}

/// Manages block time
#[derive(Clone, Debug)]
pub struct TimeManager {
    /// tracks the overall applied timestamp offset
    offset: Arc<RwLock<i128>>,
    /// The timestamp of the last block header.
    ///
    /// Interpreted as seconds
    last_timestamp: Arc<RwLock<u64>>,
    /// Contains the next timestamp to use
    /// if this is set then the next time `[TimeManager::current_timestamp()]` is called this value
    /// will be taken and returned. After which the `offset` will be updated accordingly
    ///
    /// Interpreted as seconds
    next_exact_timestamp: Arc<RwLock<Option<u64>>>,
    /// The interval to use when determining the next block's timestamp
    ///
    /// Interpreted as milliseconds
    interval: Arc<RwLock<Option<u64>>>,
    /// The wall clock timestamp with precision upto milliseconds
    ///
    /// This keeps track of the current timestamp in milliseconds, which is helpful for interval
    /// mining with < 1000ms block times.
    wall_clock_timestamp: Arc<RwLock<Option<u64>>>,
}

// === impl TimeManager ===

impl TimeManager {
    pub fn new(start_timestamp: u64) -> TimeManager {
        let time_manager = TimeManager {
            last_timestamp: Default::default(),
            offset: Default::default(),
            next_exact_timestamp: Default::default(),
            interval: Default::default(),
            wall_clock_timestamp: Default::default(),
        };
        time_manager.reset(start_timestamp);
        time_manager
    }

    /// Resets the current time manager to the given timestamp, resetting the offsets and
    /// next block timestamp option
    pub fn reset(&self, start_timestamp: u64) {
        let current = duration_since_unix_epoch().as_secs() as i128;
        *self.last_timestamp.write() = start_timestamp;
        *self.offset.write() = (start_timestamp as i128) - current;
        self.next_exact_timestamp.write().take();
        *self.wall_clock_timestamp.write() = Some(start_timestamp.saturating_mul(1000)); // Since, wall_clock_timestamp is in milliseconds
    }

    pub fn offset(&self) -> i128 {
        *self.offset.read()
    }

    /// Adds the given `offset` to the already tracked offset and returns the result
    fn add_offset(&self, offset: i128) -> i128 {
        let mut current = self.offset.write();
        let next = current.saturating_add(offset);
        trace!(target: "time", "adding timestamp offset={}, total={}", offset, next);
        *current = next;
        next
    }

    /// Jumps forward in time by the given seconds
    ///
    /// This will apply a permanent offset to the natural UNIX Epoch timestamp
    pub fn increase_time(&self, seconds: u64) -> i128 {
        self.add_offset(seconds as i128)
    }

    /// Sets the exact timestamp (`next_exact_timestamp`) to use in the next block
    /// Fails if it's before (or at the same time) the last timestamp
    pub fn set_next_block_timestamp(&self, timestamp: u64) -> Result<(), BlockchainError> {
        trace!(target: "time", "override next timestamp {}", timestamp);
        if timestamp <= *self.last_timestamp.read() {
            return Err(BlockchainError::TimestampError(format!(
                "{timestamp} is lower than or equal to previous block's timestamp"
            )))
        }
        self.next_exact_timestamp.write().replace(timestamp);
        Ok(())
    }

    /// Sets an interval (in milliseconds) to use when computing the next timestamp
    ///
    /// If an interval already exists, this will update the interval, otherwise a new interval will
    /// be set starting with the current timestamp.
    pub fn set_block_timestamp_interval(&self, interval: u64) {
        trace!(target: "time", "set interval {}", interval);
        self.interval.write().replace(interval);
    }

    /// Removes the interval if it exists
    pub fn remove_block_timestamp_interval(&self) -> bool {
        if self.interval.write().take().is_some() {
            trace!(target: "time", "removed interval");
            true
        } else {
            false
        }
    }

    /// Updates `wall_clock_timestamp` by `interval_ms`
    pub fn update_wall_clock_timestamp_by_interval(&self, interval_ms: u64) {
        let current_wall_timestamp = self.wall_clock_timestamp.read().unwrap();
        self.update_wall_clock_timestamp(current_wall_timestamp.saturating_add(interval_ms));
    }

    /// Updates `wall_clock_timestamp` to the given timestamp (milliseconds precision)
    pub fn update_wall_clock_timestamp(&self, timestamp: u64) {
        *self.wall_clock_timestamp.write() = Some(timestamp);
    }

    /// Computes the next timestamp without updating internals
    fn compute_next_timestamp(&self, update_wall: bool) -> (u64, Option<i128>) {
        let current = duration_since_unix_epoch().as_secs() as i128; // TODO(yash): Getting current time here as seconds.
        let last_timestamp = *self.last_timestamp.read();

        // TODO(yash): NOTE - interval in the TimeManager is always None even if --block-time has
        // been used.
        let interval = *self.interval.read();
        let (mut next_timestamp, update_offset) =
            if let Some(next) = *self.next_exact_timestamp.read() {
                if update_wall {
                    self.update_wall_clock_timestamp(next);
                }
                (next, true)
            } else if let Some(interval) = interval {
                let wall_clock_timestamp = if update_wall {
                    self.update_wall_clock_timestamp_by_interval(interval);
                    self.wall_clock_timestamp.read().unwrap()
                } else {
                    let current_wall = self.wall_clock_timestamp.read().unwrap();
                    current_wall.saturating_add(interval)
                };
                let next_timestamp = (wall_clock_timestamp as f64 / 1000.0).floor() as u64;

                (next_timestamp, false)
            } else {
                let next = current.saturating_add(self.offset()) as u64;
                if update_wall {
                    self.update_wall_clock_timestamp(next);
                }
                (next, false)
            };
        // Ensures that the timestamp is always increasing
        if next_timestamp < last_timestamp {
            next_timestamp = last_timestamp + 1;
        }
        let next_offset = update_offset.then_some((next_timestamp as i128) - current);
        (next_timestamp, next_offset)
    }

    /// Returns the current timestamp and updates the underlying offset and interval accordingly
    pub fn next_timestamp_as_secs(&self) -> u64 {
        let (next_timestamp, next_offset) = self.compute_next_timestamp(true);
        // Make sure we reset the `next_exact_timestamp`
        self.next_exact_timestamp.write().take();
        if let Some(next_offset) = next_offset {
            *self.offset.write() = next_offset;
        }
        *self.last_timestamp.write() = next_timestamp;
        next_timestamp
    }

    /// Returns the current timestamp for a call that does _not_ update the value
    pub fn current_call_timestamp(&self) -> u64 {
        let (next_timestamp, _) = self.compute_next_timestamp(false);
        next_timestamp
    }
}

/// Returns the current duration since unix epoch.
pub fn duration_since_unix_epoch() -> Duration {
    use std::time::SystemTime;
    let now = SystemTime::now();
    now.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|err| panic!("Current time {now:?} is invalid: {err:?}"))
}
