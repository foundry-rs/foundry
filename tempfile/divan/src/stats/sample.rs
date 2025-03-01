use std::collections::HashMap;

use crate::{
    alloc::ThreadAllocInfo,
    counter::KnownCounterKind,
    time::{FineDuration, Timer, Timestamp},
};

/// Timing measurement.
pub(crate) struct TimeSample {
    /// The time this sample took to run.
    ///
    /// This is gotten from [`RawSample`] with:
    /// `end.duration_since(start, timer).clamp_to(timer.precision())`.
    pub duration: FineDuration,
}

/// Unprocessed measurement.
///
/// This cannot be serialized because [`Timestamp`] is an implementation detail
/// for both the `Instant` and TSC timers.
pub(crate) struct RawSample {
    pub start: Timestamp,
    pub end: Timestamp,
    pub timer: Timer,
    pub alloc_info: ThreadAllocInfo,
    pub counter_totals: [u128; KnownCounterKind::COUNT],
}

impl RawSample {
    /// Simply computes `end - start` without clamping to precision.
    #[inline]
    pub fn duration(&self) -> FineDuration {
        self.end.duration_since(self.start, self.timer)
    }
}

/// Sample collection.
#[derive(Default)]
pub(crate) struct SampleCollection {
    /// The number of iterations within each sample.
    pub sample_size: u32,

    /// Collected timings.
    pub time_samples: Vec<TimeSample>,

    /// Allocation information associated with `time_samples` by index.
    pub alloc_info_by_sample: HashMap<u32, ThreadAllocInfo>,
}

impl SampleCollection {
    /// Discards all recorded data.
    #[inline]
    pub fn clear(&mut self) {
        self.time_samples.clear();
        self.alloc_info_by_sample.clear();
    }

    /// Computes the total number of iterations across all samples.
    ///
    /// We use `u64` in case sample count and sizes are huge.
    #[inline]
    pub fn iter_count(&self) -> u64 {
        self.sample_size as u64 * self.time_samples.len() as u64
    }

    /// Computes the total time across all samples.
    #[inline]
    pub fn total_duration(&self) -> FineDuration {
        FineDuration { picos: self.time_samples.iter().map(|s| s.duration.picos).sum() }
    }

    /// Returns all samples sorted by duration.
    #[inline]
    pub fn sorted_samples(&self) -> Vec<&TimeSample> {
        let mut result: Vec<&TimeSample> = self.time_samples.iter().collect();
        result.sort_unstable_by_key(|s| s.duration);
        result
    }
}
