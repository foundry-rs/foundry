//! Measurement statistics.

use crate::{
    alloc::{AllocOpMap, AllocTally},
    counter::{KnownCounterKind, MaxCountUInt},
    time::FineDuration,
};

mod sample;

pub(crate) use sample::*;

/// Statistics from samples.
pub(crate) struct Stats {
    /// Total number of samples taken.
    pub sample_count: u32,

    /// Total number of iterations (currently `sample_count * `sample_size`).
    pub iter_count: u64,

    /// Timing statistics.
    pub time: StatsSet<FineDuration>,

    /// Maximum allocated bytes and maximum number of allocations associated
    /// with the corresponding samples for `time`.
    pub max_alloc: AllocTally<StatsSet<f64>>,

    /// Allocation statistics associated with the corresponding samples for
    /// `time`.
    pub alloc_tallies: AllocOpMap<AllocTally<StatsSet<f64>>>,

    /// `Counter` counts associated with the corresponding samples for `time`.
    pub counts: [Option<StatsSet<MaxCountUInt>>; KnownCounterKind::COUNT],
}

impl Stats {
    pub fn get_counts(&self, counter_kind: KnownCounterKind) -> Option<&StatsSet<MaxCountUInt>> {
        self.counts[counter_kind as usize].as_ref()
    }
}

#[derive(Debug)]
pub(crate) struct StatsSet<T> {
    /// Associated with minimum amount of time taken by an iteration.
    pub fastest: T,

    /// Associated with maximum amount of time taken by an iteration.
    pub slowest: T,

    /// Associated with midpoint time taken by an iteration.
    pub median: T,

    /// Associated with average time taken by all iterations.
    pub mean: T,
}

impl StatsSet<f64> {
    pub fn is_zero(&self) -> bool {
        self.fastest == 0.0 && self.slowest == 0.0 && self.median == 0.0 && self.mean == 0.0
    }
}
