use crate::block_env::BlockEnvOverrides;
use alloy_primitives::{Address, B256};
use alloy_rpc_types_engine::PayloadAttributes;
use parking_lot::RwLock;
use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};

/// Manages block timestamp overrides.
#[derive(Clone, Debug)]
pub struct TimeManager {
    /// Tracks the overall applied timestamp offset.
    offset: Arc<RwLock<i128>>,
    /// The timestamp of the last mined block.
    last_timestamp: Arc<RwLock<u64>>,
    /// Contains the exact timestamp to use for the next mined block, if any.
    next_exact_timestamp: Arc<RwLock<Option<u64>>>,
    /// The interval to use when determining the next block's timestamp.
    interval: Arc<RwLock<Option<u64>>>,
    /// The wall clock time at which the last block was built.
    last_block_wall_time: Arc<RwLock<u64>>,
}

impl TimeManager {
    /// Creates a new time manager with the given start timestamp.
    pub fn new(start_timestamp: u64) -> Self {
        let time_manager = Self {
            offset: Default::default(),
            last_timestamp: Default::default(),
            next_exact_timestamp: Default::default(),
            interval: Default::default(),
            last_block_wall_time: Arc::new(RwLock::new(duration_since_unix_epoch().as_secs())),
        };
        time_manager.reset(start_timestamp);
        time_manager
    }

    /// Resets the current time manager to the given timestamp.
    pub fn reset(&self, start_timestamp: u64) {
        let current = duration_since_unix_epoch().as_secs() as i128;
        *self.last_timestamp.write() = start_timestamp;
        *self.offset.write() = (start_timestamp as i128) - current;
        self.next_exact_timestamp.write().take();
    }

    /// Sets the current time baseline.
    pub fn set_time(&self, timestamp: u64) {
        self.reset(timestamp);
    }

    /// Jumps forward in time by the given number of seconds and returns the new offset.
    pub fn increase_time(&self, seconds: u64) -> i128 {
        let mut current = self.offset.write();
        let next = current.saturating_add(seconds as i128);
        *current = next;
        next
    }

    /// Sets the exact timestamp to use in the next block.
    pub fn set_next_block_timestamp(&self, timestamp: u64) -> Result<(), String> {
        if timestamp < *self.last_timestamp.read() {
            return Err(format!("{timestamp} is lower than previous block's timestamp"));
        }
        self.next_exact_timestamp.write().replace(timestamp);
        Ok(())
    }

    /// Sets the interval to use when determining the next block's timestamp.
    pub fn set_block_timestamp_interval(&self, interval: u64) {
        self.interval.write().replace(interval);
    }

    /// Returns the configured block timestamp interval, if any.
    pub fn interval(&self) -> Option<u64> {
        *self.interval.read()
    }

    /// Removes the interval if it exists, returning whether one was removed.
    pub fn remove_block_timestamp_interval(&self) -> bool {
        self.interval.write().take().is_some()
    }

    fn compute_next_timestamp(&self) -> (u64, Option<i128>) {
        let current = duration_since_unix_epoch().as_secs() as i128;
        let last_timestamp = *self.last_timestamp.read();

        let (mut next_timestamp, update_offset, exact_timestamp) =
            if let Some(next) = *self.next_exact_timestamp.read() {
                (next, true, true)
            } else if let Some(interval) = *self.interval.read() {
                (last_timestamp.saturating_add(interval), false, false)
            } else {
                (current.saturating_add(*self.offset.read()) as u64, false, false)
            };

        if exact_timestamp {
            if next_timestamp < last_timestamp {
                next_timestamp = last_timestamp.saturating_add(1);
            }
        } else if next_timestamp <= last_timestamp {
            next_timestamp = last_timestamp.saturating_add(1);
        }

        let next_offset = update_offset.then_some((next_timestamp as i128) - current);
        (next_timestamp, next_offset)
    }

    /// Returns the next block timestamp and updates internal state.
    pub fn next_timestamp(&self) -> u64 {
        let (next_timestamp, next_offset) = self.compute_next_timestamp();
        self.next_exact_timestamp.write().take();
        if let Some(next_offset) = next_offset {
            *self.offset.write() = next_offset;
        }
        *self.last_timestamp.write() = next_timestamp;
        *self.last_block_wall_time.write() = duration_since_unix_epoch().as_secs();
        next_timestamp
    }

    /// Returns the wall clock time at which the last block was built.
    pub fn last_block_wall_time(&self) -> u64 {
        *self.last_block_wall_time.read()
    }

    /// Captures the current settings.
    pub fn snapshot(&self) -> TimeSnapshot {
        TimeSnapshot {
            offset: *self.offset.read(),
            last_timestamp: *self.last_timestamp.read(),
            next_exact_timestamp: *self.next_exact_timestamp.read(),
            interval: *self.interval.read(),
        }
    }

    /// Restores the given settings.
    pub fn restore(&self, snapshot: TimeSnapshot) {
        *self.offset.write() = snapshot.offset;
        *self.last_timestamp.write() = snapshot.last_timestamp;
        *self.next_exact_timestamp.write() = snapshot.next_exact_timestamp;
        *self.interval.write() = snapshot.interval;
    }

    /// Returns the current timestamp for read-only calls without consuming overrides.
    pub fn current_call_timestamp(&self) -> u64 {
        self.compute_next_timestamp().0
    }

    /// Returns the local miner payload attribute mapper for this time manager and the given block
    /// environment overrides. The coinbase override maps to `suggested_fee_recipient`.
    pub fn payload_attributes_hook<A: AnvilPayloadAttributes>(
        &self,
        block_env: BlockEnvOverrides,
    ) -> impl Fn(A) -> A + Send + Sync + 'static {
        let time = self.clone();
        move |mut attributes: A| {
            attributes.set_timestamp(time.next_timestamp());
            if let Some(coinbase) = block_env.coinbase() {
                attributes.set_suggested_fee_recipient(coinbase);
            }
            if let Some(prev_randao) = block_env.take_next_prev_randao() {
                attributes.set_prev_randao(prev_randao);
            }
            if let Some(root) = block_env.take_next_parent_beacon_block_root() {
                attributes.set_parent_beacon_block_root(root);
            }
            attributes
        }
    }
}

/// Payload attributes the time manager and the block environment overrides can adjust.
pub trait AnvilPayloadAttributes: Send + 'static {
    /// Sets the block timestamp.
    fn set_timestamp(&mut self, timestamp: u64);
    /// Sets the fee recipient.
    fn set_suggested_fee_recipient(&mut self, recipient: Address);
    /// Sets the prev randao.
    fn set_prev_randao(&mut self, prev_randao: B256);
    /// Sets the parent beacon block root.
    fn set_parent_beacon_block_root(&mut self, root: B256);
}

impl AnvilPayloadAttributes for PayloadAttributes {
    fn set_timestamp(&mut self, timestamp: u64) {
        self.timestamp = timestamp;
    }

    fn set_suggested_fee_recipient(&mut self, recipient: Address) {
        self.suggested_fee_recipient = recipient;
    }

    fn set_prev_randao(&mut self, prev_randao: B256) {
        self.prev_randao = prev_randao;
    }

    fn set_parent_beacon_block_root(&mut self, root: B256) {
        self.parent_beacon_block_root = Some(root);
    }
}

/// A copy of the time manager settings.
#[derive(Clone, Copy, Debug)]
pub struct TimeSnapshot {
    offset: i128,
    last_timestamp: u64,
    next_exact_timestamp: Option<u64>,
    interval: Option<u64>,
}

fn duration_since_unix_epoch() -> Duration {
    let now = SystemTime::now();
    now.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|error| panic!("current time {now:?} is invalid: {error:?}"))
}
