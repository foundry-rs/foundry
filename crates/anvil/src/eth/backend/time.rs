//! Manages the block time

use crate::eth::error::BlockchainError;
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use std::{sync::Arc, time::Duration};

/// Manages block time
#[derive(Clone, Debug)]
pub struct TimeManager {
    state: Arc<RwLock<TimeState>>,
}

/// Timestamp controls that must be read and committed atomically.
#[derive(Debug, Default)]
struct TimeState {
    offset: i128,
    /// Explicit increases not yet committed, separate from the wall-clock offset.
    time_increase: u64,
    offset_reset_generation: u64,
    last_timestamp: u64,
    last_block_wall_time: u64,
    next_exact_timestamp: Option<TimestampOverride>,
    interval: Option<u64>,
    next_override_generation: u64,
}

#[derive(Clone, Copy, Debug)]
struct TimestampOverride {
    timestamp: u64,
    generation: u64,
}

/// A timestamp prepared for a candidate block but not yet committed.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingBlockTimestamp {
    pub(crate) timestamp: u64,
    exact_generation: Option<u64>,
    prepared_offset: i128,
    offset_reset_generation: u64,
    next_offset: Option<i128>,
    time_increase: u64,
}

/// Time controls captured by an in-memory state snapshot.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TimeSnapshot {
    offset: i128,
    last_timestamp: u64,
    next_block_timestamp: Option<u64>,
    time_increase: u64,
}

/// A temporary additive time increase that can be rolled back after failed manual mining.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingTimeIncrease {
    seconds: u64,
    offset_reset_generation: u64,
}

impl TimeManager {
    pub fn new(start_timestamp: u64) -> Self {
        let time_manager = Self { state: Default::default() };
        time_manager.reset(start_timestamp);
        time_manager
    }

    /// Resets the current time manager to the given timestamp, resetting the offsets and
    /// next block timestamp option
    pub fn reset(&self, start_timestamp: u64) {
        self.reset_timestamp(start_timestamp, true, None, None, 0);
    }

    /// Sets the current timestamp without changing when the current head was installed.
    pub fn set_time(&self, timestamp: u64) {
        self.reset_timestamp(timestamp, false, None, None, 0);
    }

    /// Restores the timestamp and offset captured by a state snapshot.
    pub(crate) fn restore(&self, snapshot: TimeSnapshot) {
        self.reset_timestamp(
            snapshot.last_timestamp,
            true,
            Some(snapshot.offset),
            snapshot.next_block_timestamp,
            snapshot.time_increase,
        );
    }

    fn reset_timestamp(
        &self,
        start_timestamp: u64,
        mark_new_head: bool,
        offset: Option<i128>,
        next_block_timestamp: Option<u64>,
        time_increase: u64,
    ) {
        let current = duration_since_unix_epoch();
        let mut state = self.state.write();
        state.last_timestamp = start_timestamp;
        state.time_increase = time_increase;
        if mark_new_head {
            state.last_block_wall_time = current.as_millis().try_into().unwrap_or(u64::MAX);
        }
        state.offset =
            offset.unwrap_or_else(|| (start_timestamp as i128) - current.as_secs() as i128);
        state.offset_reset_generation = state.offset_reset_generation.wrapping_add(1);
        state.next_override_generation = state.next_override_generation.wrapping_add(1);
        state.next_exact_timestamp = next_block_timestamp.map(|timestamp| TimestampOverride {
            timestamp,
            generation: state.next_override_generation,
        });
    }

    pub fn offset(&self) -> i128 {
        self.state.read().offset
    }

    pub(crate) fn snapshot(&self) -> TimeSnapshot {
        let state = self.state.read();
        TimeSnapshot {
            offset: state.offset,
            last_timestamp: state.last_timestamp,
            next_block_timestamp: state.next_exact_timestamp.map(|override_| override_.timestamp),
            time_increase: state.time_increase,
        }
    }

    /// Returns the UNIX wall time in milliseconds when the current head was installed.
    pub fn last_block_wall_time(&self) -> u64 {
        self.state.read().last_block_wall_time
    }

    /// Records that a new latest block was created.
    pub(crate) fn mark_block_created(&self) {
        self.state.write().last_block_wall_time =
            duration_since_unix_epoch().as_millis().try_into().unwrap_or(u64::MAX);
    }

    /// Adds the given `offset` to the already tracked offset and returns the result
    fn add_offset(&self, offset: u64) -> i128 {
        let mut state = self.state.write();
        let next = state.offset.saturating_add(offset as i128);
        trace!(target: "time", "adding timestamp offset={}, total={}", offset, next);
        state.offset = next;
        state.time_increase = state.time_increase.saturating_add(offset);
        next
    }

    /// Jumps forward in time by the given seconds
    ///
    /// This will apply a permanent offset to the natural UNIX Epoch timestamp
    pub fn increase_time(&self, seconds: u64) -> i128 {
        self.add_offset(seconds)
    }

    /// Applies a temporary increase that can be rolled back if mining fails.
    pub(crate) fn apply_time_increase(&self, seconds: u64) -> PendingTimeIncrease {
        let mut state = self.state.write();
        state.offset = state.offset.saturating_add(seconds as i128);
        state.time_increase = state.time_increase.saturating_add(seconds);
        PendingTimeIncrease { seconds, offset_reset_generation: state.offset_reset_generation }
    }

    /// Reverts a temporary increase unless an absolute time reset superseded it.
    pub(crate) fn revert_time_increase(&self, pending: PendingTimeIncrease) {
        let mut state = self.state.write();
        if state.offset_reset_generation == pending.offset_reset_generation {
            state.offset = state.offset.saturating_sub(pending.seconds as i128);
            state.time_increase = state.time_increase.saturating_sub(pending.seconds);
        }
    }

    /// Sets the exact timestamp to use in the next block
    /// Fails if it's before (or at the same time) the last timestamp
    pub fn set_next_block_timestamp(&self, timestamp: u64) -> Result<(), BlockchainError> {
        trace!(target: "time", "override next timestamp {}", timestamp);
        let mut state = self.state.write();
        if timestamp < state.last_timestamp {
            return Err(BlockchainError::TimestampError(format!(
                "{timestamp} is lower than previous block's timestamp"
            )));
        }
        state.next_override_generation = state.next_override_generation.wrapping_add(1);
        state.next_exact_timestamp =
            Some(TimestampOverride { timestamp, generation: state.next_override_generation });
        Ok(())
    }

    /// Sets an interval to use when computing the next timestamp
    ///
    /// If an interval already exists, this will update the interval, otherwise a new interval will
    /// be set starting with the current timestamp.
    pub fn set_block_timestamp_interval(&self, interval: u64) {
        trace!(target: "time", "set interval {}", interval);
        self.state.write().interval = Some(interval);
    }

    /// Returns the configured block timestamp interval.
    pub(crate) fn block_timestamp_interval(&self) -> Option<u64> {
        self.state.read().interval
    }

    /// Removes the interval if it exists
    pub fn remove_block_timestamp_interval(&self) -> bool {
        if self.state.write().interval.take().is_some() {
            trace!(target: "time", "removed interval");
            true
        } else {
            false
        }
    }

    /// Computes the next timestamp without updating internals
    fn compute_next_timestamp(
        state: &TimeState,
        current: i128,
    ) -> (u64, Option<u64>, Option<i128>) {
        let exact_timestamp = state.next_exact_timestamp;
        let last_timestamp = state.last_timestamp;

        let (mut next_timestamp, update_offset) = if let Some(next) = exact_timestamp {
            (next.timestamp, true)
        } else if let Some(interval) = state.interval {
            (last_timestamp.saturating_add(interval), false)
        } else {
            (current.saturating_add(state.offset) as u64, false)
        };
        // Ensures that the timestamp is always increasing
        if next_timestamp < last_timestamp {
            next_timestamp = last_timestamp + 1;
        }
        let next_offset = update_offset.then_some((next_timestamp as i128) - current);
        (next_timestamp, exact_timestamp.map(|exact| exact.generation), next_offset)
    }

    /// Prepares the next timestamp without consuming a one-shot override.
    pub(crate) fn prepare_next_timestamp(&self) -> PendingBlockTimestamp {
        self.prepare_next_timestamp_inner(None)
    }

    /// Prepares a logical block clock, using explicit controls instead of elapsed wall time.
    ///
    /// `increment` is the whole-second carry supplied by the chain's block schedule. The
    /// minimum keeps a sub-second rollover monotonic even when an override repeats the parent.
    #[cfg(any(feature = "base", test))]
    pub(crate) fn prepare_next_timestamp_with_increment(
        &self,
        increment: u64,
        minimum: u64,
    ) -> PendingBlockTimestamp {
        self.prepare_next_timestamp_inner(Some((increment, minimum)))
    }

    fn prepare_next_timestamp_inner(&self, schedule: Option<(u64, u64)>) -> PendingBlockTimestamp {
        let current = duration_since_unix_epoch().as_secs() as i128;
        let state = self.state.read();
        let (timestamp, exact_generation, next_offset) =
            if let Some((increment, minimum)) = schedule {
                let timestamp = state
                    .next_exact_timestamp
                    .map_or_else(
                        || {
                            state
                                .last_timestamp
                                .saturating_add(state.interval.unwrap_or(increment))
                                .saturating_add(state.time_increase)
                        },
                        |next| next.timestamp,
                    )
                    .max(minimum);
                (
                    timestamp,
                    state.next_exact_timestamp.map(|next| next.generation),
                    Some(timestamp as i128 - current),
                )
            } else {
                Self::compute_next_timestamp(&state, current)
            };
        PendingBlockTimestamp {
            timestamp,
            exact_generation,
            prepared_offset: state.offset,
            offset_reset_generation: state.offset_reset_generation,
            next_offset,
            time_increase: state.time_increase,
        }
    }

    /// Commits a timestamp after its candidate block finalized successfully.
    pub(crate) fn commit_next_timestamp(&self, pending: PendingBlockTimestamp) {
        let mut state = self.state.write();
        if pending.exact_generation.is_some_and(|generation| {
            state.next_exact_timestamp.is_some_and(|exact| exact.generation == generation)
        }) {
            state.next_exact_timestamp = None;
        }
        if let Some(next_offset) = pending.next_offset
            && state.offset_reset_generation == pending.offset_reset_generation
        {
            let concurrent_offset = state.offset.saturating_sub(pending.prepared_offset);
            state.offset = next_offset.saturating_add(concurrent_offset);
        }
        if state.offset_reset_generation == pending.offset_reset_generation {
            state.time_increase = state.time_increase.saturating_sub(pending.time_increase);
            state.last_timestamp = pending.timestamp;
        }
    }

    /// Returns the current timestamp and updates the underlying offset and interval accordingly
    pub fn next_timestamp(&self) -> u64 {
        let pending = self.prepare_next_timestamp();
        self.commit_next_timestamp(pending);
        pending.timestamp
    }

    /// Returns the current timestamp for a call that does _not_ update the value
    pub fn current_call_timestamp(&self) -> u64 {
        self.prepare_next_timestamp().timestamp
    }
}

/// Returns the `Utc` datetime for the given seconds since unix epoch
pub fn utc_from_secs(secs: u64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs as i64, 0).unwrap_or(DateTime::<Utc>::MAX_UTC)
}

/// Returns the current duration since unix epoch.
pub fn duration_since_unix_epoch() -> Duration {
    use std::time::SystemTime;
    let now = SystemTime::now();
    now.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|err| panic!("Current time {now:?} is invalid: {err:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_consumes_only_its_timestamp_override() {
        let time = TimeManager::new(1);
        time.set_next_block_timestamp(100).unwrap();
        let pending = time.prepare_next_timestamp();

        time.set_next_block_timestamp(100).unwrap();
        time.commit_next_timestamp(pending);

        let state = time.state.read();
        assert_eq!(state.last_timestamp, 100);
        assert_eq!(state.next_exact_timestamp.unwrap().timestamp, 100);
    }

    #[test]
    fn candidate_commit_preserves_concurrent_time_increase() {
        let time = TimeManager::new(1);
        time.set_next_block_timestamp(100).unwrap();
        let pending = time.prepare_next_timestamp();

        time.increase_time(10);
        time.commit_next_timestamp(pending);

        let state = time.state.read();
        assert_eq!(state.last_timestamp, 100);
        assert_eq!(state.offset, pending.next_offset.unwrap() + 10);
    }

    #[test]
    fn candidate_commit_preserves_concurrent_time_reset() {
        let time = TimeManager::new(1);
        time.set_next_block_timestamp(100).unwrap();
        let pending = time.prepare_next_timestamp();

        time.reset(1_000);
        let reset_offset = time.offset();
        time.commit_next_timestamp(pending);

        let state = time.state.read();
        assert_eq!(state.last_timestamp, 1_000);
        assert_eq!(state.offset, reset_offset);
    }

    #[test]
    fn failed_temporary_increase_preserves_concurrent_time_reset() {
        let time = TimeManager::new(1);
        let pending = time.apply_time_increase(60);

        time.reset(1_000);
        let reset_offset = time.offset();
        time.revert_time_increase(pending);

        assert_eq!(time.offset(), reset_offset);
    }

    #[test]
    fn logical_clock_ignores_wall_time_and_preserves_explicit_controls() {
        let time = TimeManager::new(100);
        // Move the wall-clock fallback without issuing an explicit time increase.
        time.state.write().offset += 1_000;
        assert!(time.current_call_timestamp() >= 1_100);
        assert_eq!(time.prepare_next_timestamp_with_increment(0, 100).timestamp, 100);

        time.increase_time(10);
        let pending = time.prepare_next_timestamp_with_increment(0, 100);
        assert_eq!(pending.timestamp, 110);
        assert_eq!(time.prepare_next_timestamp_with_increment(0, 100).timestamp, 110);
        time.increase_time(7);
        time.commit_next_timestamp(pending);
        let pending = time.prepare_next_timestamp_with_increment(1, 111);
        assert_eq!(pending.timestamp, 118);
        time.commit_next_timestamp(pending);
        assert_eq!(time.prepare_next_timestamp_with_increment(0, 118).timestamp, 118);

        time.set_block_timestamp_interval(2);
        assert_eq!(time.prepare_next_timestamp_with_increment(0, 118).timestamp, 120);
        time.set_next_block_timestamp(200).unwrap();
        let pending = time.prepare_next_timestamp_with_increment(0, 118);
        assert_eq!(pending.timestamp, 200);
        time.set_next_block_timestamp(300).unwrap();
        time.commit_next_timestamp(pending);
        assert_eq!(time.prepare_next_timestamp_with_increment(0, 200).timestamp, 300);
    }

    #[test]
    fn logical_clock_restores_pending_increases_and_absolute_resets() {
        let time = TimeManager::new(100);
        time.set_time(500);
        time.increase_time(10);
        let snapshot = time.snapshot();
        time.commit_next_timestamp(time.prepare_next_timestamp_with_increment(0, 100));
        time.restore(snapshot);
        assert_eq!(time.prepare_next_timestamp_with_increment(0, 100).timestamp, 510);

        let pending = time.prepare_next_timestamp_with_increment(0, 100);
        time.set_time(1_000);
        time.increase_time(20);
        time.commit_next_timestamp(pending);
        assert_eq!(time.prepare_next_timestamp_with_increment(0, 510).timestamp, 1_020);
    }

    #[test]
    fn logical_clock_reverts_failed_manual_time_increase() {
        let time = TimeManager::new(100);
        let increase = time.apply_time_increase(60);
        assert_eq!(time.prepare_next_timestamp_with_increment(0, 100).timestamp, 160);
        time.revert_time_increase(increase);
        assert_eq!(time.prepare_next_timestamp_with_increment(0, 100).timestamp, 100);
        time.set_next_block_timestamp(100).unwrap();
        assert_eq!(time.prepare_next_timestamp_with_increment(1, 101).timestamp, 101);
    }
}
