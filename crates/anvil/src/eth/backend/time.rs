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
    /// The timestamp of the last block header
    last_timestamp: Arc<RwLock<u64>>,
    /// Contains the next timestamp to use
    /// if this is set then the next time `[TimeManager::current_timestamp()]` is called this value
    /// will be taken and returned. After which the `offset` will be updated accordingly
    next_exact_timestamp: Arc<RwLock<Option<u64>>>,
    /// The interval to use when determining the next block's timestamp
    interval: Arc<RwLock<Option<u64>>>,
}

impl TimeManager {
    pub fn new(start_timestamp: u64) -> Self {
        let time_manager = Self {
            last_timestamp: Default::default(),
            offset: Default::default(),
            next_exact_timestamp: Default::default(),
            interval: Default::default(),
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

    /// Sets the exact timestamp to use in the next block
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

    /// Sets an interval to use when computing the next timestamp
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

    /// Computes the next timestamp without updating internals
    fn compute_next_timestamp(&self) -> (u64, Option<i128>) {
        let current = duration_since_unix_epoch().as_secs() as i128;
        let last_timestamp = *self.last_timestamp.read();

        let (mut next_timestamp, update_offset) =
            if let Some(next) = *self.next_exact_timestamp.read() {
                (next, true)
            } else if let Some(interval) = *self.interval.read() {
                (last_timestamp.saturating_add(interval), false)
            } else {
                (current.saturating_add(self.offset()) as u64, false)
            };
        // Ensures that the timestamp is always increasing
        if next_timestamp <= last_timestamp {
            next_timestamp = last_timestamp + 1;
        }
        let next_offset = update_offset.then_some((next_timestamp as i128) - current);
        (next_timestamp, next_offset)
    }

    /// Returns the current timestamp and updates the underlying offset and interval accordingly
    pub fn next_timestamp(&self) -> u64 {
        let (next_timestamp, next_offset) = self.compute_next_timestamp();
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
        let (next_timestamp, _) = self.compute_next_timestamp();
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
