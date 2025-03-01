use std::{borrow::Cow, time::Duration};

use crate::{counter::CounterSet, time::FineDuration};

/// Benchmarking options set directly by the user in `#[divan::bench]` and
/// `#[divan::bench_group]`.
///
/// Changes to fields must be reflected in the "Options" sections of the docs
/// for `#[divan::bench]` and `#[divan::bench_group]`.
#[derive(Clone, Default)]
pub struct BenchOptions<'a> {
    /// The number of sample recordings.
    pub sample_count: Option<u32>,

    /// The number of iterations inside a single sample.
    pub sample_size: Option<u32>,

    /// The number of threads to benchmark the sample. This is 1 by default.
    ///
    /// If set to 0, this will use [`std::thread::available_parallelism`].
    ///
    /// We use `&'static [usize]` by leaking the input because `BenchOptions` is
    /// cached on first retrieval.
    pub threads: Option<Cow<'a, [usize]>>,

    /// Counts the number of values processed each iteration of a benchmarked
    /// function.
    pub counters: CounterSet,

    /// The time floor for benchmarking a function.
    pub min_time: Option<Duration>,

    /// The time ceiling for benchmarking a function.
    pub max_time: Option<Duration>,

    /// When accounting for `min_time` or `max_time`, skip time external to
    /// benchmarked functions, such as time spent generating inputs and running
    /// [`Drop`].
    pub skip_ext_time: Option<bool>,

    /// Whether the benchmark should be ignored.
    ///
    /// This may be set within the attribute or with a separate
    /// [`#[ignore]`](https://doc.rust-lang.org/reference/attributes/testing.html#the-ignore-attribute).
    pub ignore: Option<bool>,
}

impl<'a> BenchOptions<'a> {
    /// Overwrites `other` with values set in `self`.
    #[must_use]
    pub(crate) fn overwrite<'b>(&'b self, other: &'b Self) -> Self
    where
        'b: 'a,
    {
        Self {
            // `Copy` values:
            sample_count: self.sample_count.or(other.sample_count),
            sample_size: self.sample_size.or(other.sample_size),
            threads: self.threads.as_deref().or(other.threads.as_deref()).map(Cow::Borrowed),
            min_time: self.min_time.or(other.min_time),
            max_time: self.max_time.or(other.max_time),
            skip_ext_time: self.skip_ext_time.or(other.skip_ext_time),
            ignore: self.ignore.or(other.ignore),

            // `Clone` values:
            counters: self.counters.overwrite(&other.counters),
        }
    }

    /// Returns `true` if non-zero samples are specified.
    #[inline]
    pub(crate) fn has_samples(&self) -> bool {
        self.sample_count != Some(0) && self.sample_size != Some(0)
    }

    #[inline]
    pub(crate) fn min_time(&self) -> FineDuration {
        self.min_time.map(FineDuration::from).unwrap_or_default()
    }

    #[inline]
    pub(crate) fn max_time(&self) -> FineDuration {
        self.max_time.map(FineDuration::from).unwrap_or(FineDuration::MAX)
    }
}
