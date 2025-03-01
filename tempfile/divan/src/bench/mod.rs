use std::{
    cell::UnsafeCell,
    fmt,
    mem::{self, MaybeUninit},
    num::NonZeroUsize,
    sync::Barrier,
};

use crate::{
    alloc::{
        AllocOp, AllocOpMap, AllocTally, ThreadAllocInfo, ThreadAllocTally, TotalAllocTallyMap,
    },
    black_box, black_box_drop,
    counter::{
        AnyCounter, AsCountUInt, BytesCount, CharsCount, Counter, CounterCollection, CyclesCount,
        IntoCounter, ItemsCount, KnownCounterKind, MaxCountUInt,
    },
    divan::SharedContext,
    stats::{RawSample, SampleCollection, Stats, StatsSet, TimeSample},
    thread_pool::BENCH_POOL,
    time::{FineDuration, Timestamp, UntaggedTimestamp},
    util::{self, sync::SyncWrap, Unit},
};

#[cfg(test)]
mod tests;

mod args;
mod defer;
mod options;

use defer::{DeferSlot, DeferStore};

pub use self::{
    args::{BenchArgs, BenchArgsRunner},
    options::BenchOptions,
};

pub(crate) const DEFAULT_SAMPLE_COUNT: u32 = 100;

/// Enables contextual benchmarking in [`#[divan::bench]`](attr.bench.html).
///
/// # Examples
///
/// ```
/// use divan::{Bencher, black_box};
///
/// #[divan::bench]
/// fn copy_from_slice(bencher: Bencher) {
///     // Input and output buffers get used in the closure.
///     let src = (0..100).collect::<Vec<i32>>();
///     let mut dst = vec![0; src.len()];
///
///     bencher.bench_local(|| {
///         black_box(&mut dst).copy_from_slice(black_box(&src));
///     });
/// }
/// ```
#[must_use = "a benchmark function must be registered"]
pub struct Bencher<'a, 'b, C = BencherConfig> {
    pub(crate) context: &'a mut BenchContext<'b>,
    pub(crate) config: C,
}

/// Public-in-private type for statically-typed `Bencher` configuration.
///
/// This enables configuring `Bencher` using the builder pattern with zero
/// runtime cost.
pub struct BencherConfig<GenI = Unit> {
    gen_input: GenI,
}

impl<C> fmt::Debug for Bencher<'_, '_, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bencher").finish_non_exhaustive()
    }
}

impl<'a, 'b> Bencher<'a, 'b> {
    #[inline]
    pub(crate) fn new(context: &'a mut BenchContext<'b>) -> Self {
        Self { context, config: BencherConfig { gen_input: Unit } }
    }
}

impl<'a, 'b> Bencher<'a, 'b> {
    /// Benchmarks a function.
    ///
    /// The function can be benchmarked in parallel using the [`threads`
    /// option](macro@crate::bench#threads). If the function is strictly
    /// single-threaded, use [`Bencher::bench_local`] instead.
    ///
    /// # Examples
    ///
    /// ```
    /// #[divan::bench]
    /// fn bench(bencher: divan::Bencher) {
    ///     bencher.bench(|| {
    ///         // Benchmarked code...
    ///     });
    /// }
    /// ```
    pub fn bench<O, B>(self, benched: B)
    where
        B: Fn() -> O + Sync,
    {
        // Reusing `bench_values` for a zero-sized non-drop input type should
        // have no overhead.
        self.with_inputs(|| ()).bench_values(|_: ()| benched());
    }

    /// Benchmarks a function on the current thread.
    ///
    /// # Examples
    ///
    /// ```
    /// #[divan::bench]
    /// fn bench(bencher: divan::Bencher) {
    ///     bencher.bench_local(|| {
    ///         // Benchmarked code...
    ///     });
    /// }
    /// ```
    pub fn bench_local<O, B>(self, mut benched: B)
    where
        B: FnMut() -> O,
    {
        // Reusing `bench_local_values` for a zero-sized non-drop input type
        // should have no overhead.
        self.with_inputs(|| ()).bench_local_values(|_: ()| benched());
    }

    /// Generate inputs for the [benchmarked function](#input-bench).
    ///
    /// Time spent generating inputs does not affect benchmark timing.
    ///
    /// When [benchmarking in parallel](macro@crate::bench#threads), the input
    /// generator is called on the same thread as the sample loop that uses that
    /// input.
    ///
    /// # Examples
    ///
    /// ```
    /// #[divan::bench]
    /// fn bench(bencher: divan::Bencher) {
    ///     bencher
    ///         .with_inputs(|| {
    ///             // Generate input:
    ///             String::from("...")
    ///         })
    ///         .bench_values(|s| {
    ///             // Use input by-value:
    ///             s + "123"
    ///         });
    /// }
    /// ```
    pub fn with_inputs<G>(self, gen_input: G) -> Bencher<'a, 'b, BencherConfig<G>> {
        Bencher { context: self.context, config: BencherConfig { gen_input } }
    }
}

impl<'a, 'b, GenI> Bencher<'a, 'b, BencherConfig<GenI>> {
    /// Assign a [`Counter`] for all iterations of the benchmarked function.
    ///
    /// This will either:
    /// - Assign a new counter
    /// - Override an existing counter of the same type
    ///
    /// If the counter depends on [generated inputs](Self::with_inputs), use
    /// [`Bencher::input_counter`] instead.
    ///
    /// If context is not needed, the counter can instead be set via
    /// [`#[divan::bench(counters = ...)]`](macro@crate::bench#counters).
    ///
    /// # Examples
    ///
    /// ```
    /// use divan::{Bencher, counter::BytesCount};
    ///
    /// #[divan::bench]
    /// fn char_count(bencher: Bencher) {
    ///     let s: String = // ...
    ///     # String::new();
    ///
    ///     bencher
    ///         .counter(BytesCount::of_str(&s))
    ///         .bench(|| {
    ///             divan::black_box(&s).chars().count()
    ///         });
    /// }
    /// ```
    #[doc(alias = "throughput")]
    pub fn counter<C>(self, counter: C) -> Self
    where
        C: IntoCounter,
    {
        let counter = AnyCounter::new(counter);
        self.context.counters.set_counter(counter);
        self
    }
}

/// <span id="input-bench"></span> Benchmark over [generated inputs](Self::with_inputs).
impl<'a, 'b, I, GenI> Bencher<'a, 'b, BencherConfig<GenI>>
where
    GenI: FnMut() -> I,
{
    /// Calls a closure to create a [`Counter`] for each input of the
    /// benchmarked function.
    ///
    /// This will either:
    /// - Assign a new counter
    /// - Override an existing counter of the same type
    ///
    /// If the counter is constant, use [`Bencher::counter`] instead.
    ///
    /// When [benchmarking in parallel](macro@crate::bench#threads), the input
    /// counter is called on the same thread as the sample loop that generates
    /// and uses that input.
    ///
    /// # Examples
    ///
    /// The following example emits info for the number of bytes processed when
    /// benchmarking [`char`-counting](std::str::Chars::count). The byte count
    /// is gotten by calling [`BytesCount::of_str`] on each iteration's input
    /// [`String`].
    ///
    /// ```
    /// use divan::{Bencher, counter::BytesCount};
    ///
    /// #[divan::bench]
    /// fn char_count(bencher: Bencher) {
    ///     bencher
    ///         .with_inputs(|| -> String {
    ///             // ...
    ///             # String::new()
    ///         })
    ///         .input_counter(BytesCount::of_str)
    ///         .bench_refs(|s| {
    ///             s.chars().count()
    ///         });
    /// }
    /// ```
    pub fn input_counter<C, F>(self, make_counter: F) -> Self
    where
        F: Fn(&I) -> C + Sync + 'static,
        C: IntoCounter,
    {
        self.context.counters.set_input_counter(make_counter);
        self
    }

    /// Creates a [`Counter`] from each input of the benchmarked function.
    ///
    /// This may be used if the input returns [`u8`]–[`u64`], [`usize`], or any
    /// nesting of references to those types.
    ///
    /// # Examples
    ///
    /// The following example emits info for the number of items processed when
    /// benchmarking [`FromIterator`] from
    /// <code>[Range](std::ops::Range)<[usize]></code> to [`Vec`].
    ///
    /// ```
    /// use divan::{Bencher, counter::ItemsCount};
    ///
    /// #[divan::bench]
    /// fn range_to_vec(bencher: Bencher) {
    ///     bencher
    ///         .with_inputs(|| -> usize {
    ///             // ...
    ///             # 0
    ///         })
    ///         .count_inputs_as::<ItemsCount>()
    ///         .bench_values(|n| -> Vec<usize> {
    ///             (0..n).collect()
    ///         });
    /// }
    /// ```
    #[inline]
    pub fn count_inputs_as<C>(self) -> Self
    where
        C: Counter,
        I: AsCountUInt,
    {
        match KnownCounterKind::of::<C>() {
            KnownCounterKind::Bytes => self.input_counter(|c| BytesCount::from(c)),
            KnownCounterKind::Chars => self.input_counter(|c| CharsCount::from(c)),
            KnownCounterKind::Cycles => self.input_counter(|c| CyclesCount::from(c)),
            KnownCounterKind::Items => self.input_counter(|c| ItemsCount::from(c)),
        }
    }

    /// Benchmarks a function over per-iteration [generated inputs](Self::with_inputs),
    /// provided by-value.
    ///
    /// Per-iteration means the benchmarked function is called exactly once for
    /// each generated input.
    ///
    /// The function can be benchmarked in parallel using the [`threads`
    /// option](macro@crate::bench#threads). If the function is strictly
    /// single-threaded, use [`Bencher::bench_local_values`] instead.
    ///
    /// # Examples
    ///
    /// ```
    /// #[divan::bench]
    /// fn bench(bencher: divan::Bencher) {
    ///     bencher
    ///         .with_inputs(|| {
    ///             // Generate input:
    ///             String::from("...")
    ///         })
    ///         .bench_values(|s| {
    ///             // Use input by-value:
    ///             s + "123"
    ///         });
    /// }
    /// ```
    pub fn bench_values<O, B>(self, benched: B)
    where
        B: Fn(I) -> O + Sync,
        GenI: Fn() -> I + Sync,
    {
        self.context.bench_loop_threaded(
            self.config.gen_input,
            |input| {
                // SAFETY: Input is guaranteed to be initialized and not
                // currently referenced by anything else.
                let input = unsafe { input.get().read().assume_init() };

                benched(input)
            },
            // Input ownership is transferred to `benched`.
            |_input| {},
        );
    }

    /// Benchmarks a function over per-iteration [generated inputs](Self::with_inputs),
    /// provided by-value.
    ///
    /// Per-iteration means the benchmarked function is called exactly once for
    /// each generated input.
    ///
    /// # Examples
    ///
    /// ```
    /// #[divan::bench]
    /// fn bench(bencher: divan::Bencher) {
    ///     let mut values = Vec::new();
    ///     bencher
    ///         .with_inputs(|| {
    ///             // Generate input:
    ///             String::from("...")
    ///         })
    ///         .bench_local_values(|s| {
    ///             // Use input by-value:
    ///             values.push(s);
    ///         });
    /// }
    /// ```
    pub fn bench_local_values<O, B>(self, mut benched: B)
    where
        B: FnMut(I) -> O,
    {
        self.context.bench_loop_local(
            self.config.gen_input,
            |input| {
                // SAFETY: Input is guaranteed to be initialized and not
                // currently referenced by anything else.
                let input = unsafe { input.get().read().assume_init() };

                benched(input)
            },
            // Input ownership is transferred to `benched`.
            |_input| {},
        );
    }

    /// Benchmarks a function over per-iteration [generated inputs](Self::with_inputs),
    /// provided by-reference.
    ///
    /// Per-iteration means the benchmarked function is called exactly once for
    /// each generated input.
    ///
    /// # Examples
    ///
    /// ```
    /// #[divan::bench]
    /// fn bench(bencher: divan::Bencher) {
    ///     bencher
    ///         .with_inputs(|| {
    ///             // Generate input:
    ///             String::from("...")
    ///         })
    ///         .bench_refs(|s| {
    ///             // Use input by-reference:
    ///             *s += "123";
    ///         });
    /// }
    /// ```
    pub fn bench_refs<O, B>(self, benched: B)
    where
        B: Fn(&mut I) -> O + Sync,
        GenI: Fn() -> I + Sync,
    {
        // TODO: Allow `O` to reference `&mut I` as long as `I` outlives `O`.
        self.context.bench_loop_threaded(
            self.config.gen_input,
            |input| {
                // SAFETY: Input is guaranteed to be initialized and not
                // currently referenced by anything else.
                let input = unsafe { (*input.get()).assume_init_mut() };

                benched(input)
            },
            // Input ownership was not transferred to `benched`.
            |input| {
                // SAFETY: This function is called after `benched` outputs are
                // dropped, so we have exclusive access.
                unsafe { (*input.get()).assume_init_drop() }
            },
        );
    }

    /// Benchmarks a function over per-iteration [generated inputs](Self::with_inputs),
    /// provided by-reference.
    ///
    /// Per-iteration means the benchmarked function is called exactly once for
    /// each generated input.
    ///
    /// # Examples
    ///
    /// ```
    /// #[divan::bench]
    /// fn bench(bencher: divan::Bencher) {
    ///     bencher
    ///         .with_inputs(|| {
    ///             // Generate input:
    ///             String::from("...")
    ///         })
    ///         .bench_local_refs(|s| {
    ///             // Use input by-reference:
    ///             *s += "123";
    ///         });
    /// }
    /// ```
    pub fn bench_local_refs<O, B>(self, mut benched: B)
    where
        B: FnMut(&mut I) -> O,
    {
        // TODO: Allow `O` to reference `&mut I` as long as `I` outlives `O`.
        self.context.bench_loop_local(
            self.config.gen_input,
            |input| {
                // SAFETY: Input is guaranteed to be initialized and not
                // currently referenced by anything else.
                let input = unsafe { (*input.get()).assume_init_mut() };

                benched(input)
            },
            // Input ownership was not transferred to `benched`.
            |input| {
                // SAFETY: This function is called after `benched` outputs are
                // dropped, so we have exclusive access.
                unsafe { (*input.get()).assume_init_drop() }
            },
        );
    }
}

/// State machine for how the benchmark is being run.
#[derive(Clone, Copy)]
pub(crate) enum BenchMode {
    /// The benchmark is being run as `--test`.
    ///
    /// Don't collect samples and run exactly once.
    Test,

    /// Scale `sample_size` to determine the right size for collecting.
    Tune { sample_size: u32 },

    /// Simply collect samples.
    Collect { sample_size: u32 },
}

impl BenchMode {
    #[inline]
    pub fn is_test(self) -> bool {
        matches!(self, Self::Test)
    }

    #[inline]
    pub fn is_tune(self) -> bool {
        matches!(self, Self::Tune { .. })
    }

    #[inline]
    pub fn is_collect(self) -> bool {
        matches!(self, Self::Collect { .. })
    }

    #[inline]
    pub fn sample_size(self) -> u32 {
        match self {
            Self::Test => 1,
            Self::Tune { sample_size, .. } | Self::Collect { sample_size, .. } => sample_size,
        }
    }
}

/// `#[divan::bench]` loop context.
///
/// Functions called within the benchmark loop should be `#[inline(always)]` to
/// ensure instruction cache locality.
pub(crate) struct BenchContext<'a> {
    shared_context: &'a SharedContext,

    /// User-configured options.
    pub options: &'a BenchOptions<'a>,

    /// Whether the benchmark loop was started.
    pub did_run: bool,

    /// The number of threads to run the benchmark. The default is 1.
    ///
    /// When set to 1, the benchmark loop is guaranteed to stay on the current
    /// thread and not spawn any threads.
    pub thread_count: NonZeroUsize,

    /// Recorded samples.
    samples: SampleCollection,

    /// Per-iteration counters grouped by sample.
    counters: CounterCollection,
}

impl<'a> BenchContext<'a> {
    /// Creates a new benchmarking context.
    pub fn new(
        shared_context: &'a SharedContext,
        options: &'a BenchOptions,
        thread_count: NonZeroUsize,
    ) -> Self {
        Self {
            shared_context,
            options,
            thread_count,
            did_run: false,
            samples: SampleCollection::default(),
            counters: options.counters.to_collection(),
        }
    }

    /// Runs the single-threaded loop for benchmarking `benched`.
    ///
    /// # Safety
    ///
    /// See `bench_loop_threaded`.
    pub fn bench_loop_local<I, O>(
        &mut self,
        gen_input: impl FnMut() -> I,
        benched: impl FnMut(&UnsafeCell<MaybeUninit<I>>) -> O,
        drop_input: impl Fn(&UnsafeCell<MaybeUninit<I>>),
    ) {
        // SAFETY: Closures are guaranteed to run on the current thread, so they
        // can safely be mutable and non-`Sync`.
        unsafe {
            let gen_input = SyncWrap::new(UnsafeCell::new(gen_input));
            let benched = SyncWrap::new(UnsafeCell::new(benched));
            let drop_input = SyncWrap::new(drop_input);

            self.thread_count = NonZeroUsize::MIN;
            self.bench_loop_threaded::<I, O>(
                || (*gen_input.get())(),
                |input| (*benched.get())(input),
                |input| drop_input(input),
            )
        }
    }

    /// Runs the multi-threaded loop for benchmarking `benched`.
    ///
    /// # Safety
    ///
    /// If `self.threads` is 1, the incoming closures will not escape the
    /// current thread. This guarantee ensures `bench_loop_local` can soundly
    /// reuse this method with mutable non-`Sync` closures.
    ///
    /// When `benched` is called:
    /// - `I` is guaranteed to be initialized.
    /// - No external `&I` or `&mut I` exists.
    ///
    /// When `drop_input` is called:
    /// - All instances of `O` returned from `benched` have been dropped.
    /// - The same guarantees for `I` apply as in `benched`, unless `benched`
    ///   escaped references to `I`.
    fn bench_loop_threaded<I, O>(
        &mut self,
        gen_input: impl Fn() -> I + Sync,
        benched: impl Fn(&UnsafeCell<MaybeUninit<I>>) -> O + Sync,
        drop_input: impl Fn(&UnsafeCell<MaybeUninit<I>>) + Sync,
    ) {
        self.did_run = true;

        let mut current_mode = self.initial_mode();
        let is_test = current_mode.is_test();

        let record_sample = self.sample_recorder(gen_input, benched, drop_input);

        let thread_count = self.thread_count.get();
        let aux_thread_count = thread_count - 1;

        let is_single_thread = aux_thread_count == 0;

        // Per-thread sample info returned by `record_sample`. These are
        // processed locally to emit user-facing sample info. As a result, this
        // only contains `thread_count` many elements at a time.
        let mut raw_samples = Vec::<Option<RawSample>>::new();

        // The time spent benchmarking, in picoseconds.
        //
        // Unless `skip_ext_time` is set, this includes time external to
        // `benched`, such as time spent generating inputs and running drop.
        let mut elapsed_picos: u128 = 0;

        // The minimum time for benchmarking, in picoseconds.
        let min_picos = self.options.min_time().picos;

        // The remaining time left for benchmarking, in picoseconds.
        let max_picos = self.options.max_time().picos;

        // Don't bother running if user specifies 0 max time or 0 samples.
        if max_picos == 0 || !self.options.has_samples() {
            return;
        }

        let timer = self.shared_context.timer;
        let timer_kind = timer.kind();

        let mut rem_samples = if current_mode.is_collect() {
            Some(self.options.sample_count.unwrap_or(DEFAULT_SAMPLE_COUNT))
        } else {
            None
        };

        // Only measure precision if we need to tune sample size.
        let timer_precision =
            if current_mode.is_tune() { timer.precision() } else { FineDuration::default() };

        if !is_test {
            self.samples.time_samples.reserve(self.options.sample_count.unwrap_or(1) as usize);
        }

        let skip_ext_time = self.options.skip_ext_time.unwrap_or_default();
        let initial_start = if skip_ext_time { None } else { Some(Timestamp::start(timer_kind)) };

        let bench_overheads = timer.bench_overheads();

        while {
            // Conditions for when sampling is over:
            if elapsed_picos >= max_picos {
                // Depleted the benchmarking time budget. This is a strict
                // condition regardless of sample count and minimum time.
                false
            } else if rem_samples.unwrap_or(1) > 0 {
                // More samples expected.
                true
            } else {
                // Continue if we haven't reached the time floor.
                elapsed_picos < min_picos
            }
        } {
            let sample_size = current_mode.sample_size();
            self.samples.sample_size = sample_size;

            let barrier = if is_single_thread { None } else { Some(Barrier::new(thread_count)) };

            // Sample loop helper:
            let record_sample = || -> RawSample {
                let mut counter_totals: [u128; KnownCounterKind::COUNT] =
                    [0; KnownCounterKind::COUNT];

                // Updates per-input counter info for this sample.
                let mut count_input = |input: &I| {
                    for counter_kind in KnownCounterKind::ALL {
                        // SAFETY: The `I` type cannot change since `with_inputs`
                        // cannot be called more than once on the same `Bencher`.
                        if let Some(count) =
                            unsafe { self.counters.get_input_count(counter_kind, input) }
                        {
                            let total = &mut counter_totals[counter_kind as usize];
                            *total = (*total).saturating_add(count as u128);
                        }
                    }
                };

                // Sample loop:
                let ([start, end], alloc_info) =
                    record_sample(sample_size as usize, barrier.as_ref(), &mut count_input);

                RawSample { start, end, timer, alloc_info, counter_totals }
            };

            // Sample loop:
            raw_samples.clear();
            BENCH_POOL.par_extend(&mut raw_samples, aux_thread_count, |_| record_sample());

            // Convert `&[Option<RawSample>]` to `&[Sample]`.
            let raw_samples: &[RawSample] = {
                if let Some(thread) = raw_samples
                    .iter()
                    .enumerate()
                    .find_map(|(thread, sample)| sample.is_none().then_some(thread))
                {
                    panic!("Divan benchmarking thread {thread} panicked");
                }

                unsafe {
                    assert_eq!(mem::size_of::<RawSample>(), mem::size_of::<Option<RawSample>>());
                    std::slice::from_raw_parts(raw_samples.as_ptr().cast(), raw_samples.len())
                }
            };

            // If testing, exit the benchmarking loop immediately after timing a
            // single run.
            if is_test {
                break;
            }

            let slowest_sample = raw_samples.iter().max_by_key(|s| s.duration()).unwrap();
            let slowest_time = slowest_sample.duration();

            // TODO: Make tuning be less influenced by early runs. Currently if
            // early runs are very quick but later runs are slow, benchmarking
            // will take a very long time.
            //
            // TODO: Make `sample_size` consider time generating inputs and
            // dropping inputs/outputs. Currently benchmarks like
            // `Bencher::bench_refs(String::clear)` take a very long time.
            if current_mode.is_tune() {
                // Clear previous smaller samples.
                self.samples.clear();
                self.counters.clear_input_counts();

                // If within 100x timer precision, continue tuning.
                let precision_multiple = slowest_time.picos / timer_precision.picos;
                if precision_multiple <= 100 {
                    current_mode = BenchMode::Tune { sample_size: sample_size * 2 };
                } else {
                    current_mode = BenchMode::Collect { sample_size };
                    rem_samples = Some(self.options.sample_count.unwrap_or(DEFAULT_SAMPLE_COUNT));
                }
            }

            // Returns the sample's duration adjusted for overhead.
            let sample_duration_sub_overhead = |raw_sample: &RawSample| {
                let overhead = bench_overheads.total_overhead(sample_size, &raw_sample.alloc_info);

                FineDuration {
                    picos: raw_sample
                        .duration()
                        .clamp_to(timer_precision)
                        .picos
                        .saturating_sub(overhead.picos),
                }
                .clamp_to(timer_precision)
            };

            for raw_sample in raw_samples {
                let sample_index = self.samples.time_samples.len();

                self.samples
                    .time_samples
                    .push(TimeSample { duration: sample_duration_sub_overhead(raw_sample) });

                if !raw_sample.alloc_info.tallies.is_empty() {
                    self.samples
                        .alloc_info_by_sample
                        .insert(sample_index as u32, raw_sample.alloc_info.clone());
                }

                // Insert per-input counter information.
                for counter_kind in KnownCounterKind::ALL {
                    if !self.counters.uses_input_counts(counter_kind) {
                        continue;
                    }

                    let total_count = raw_sample.counter_totals[counter_kind as usize];

                    // Cannot overflow `MaxCountUInt` because `total_count`
                    // cannot exceed `MaxCountUInt::MAX * sample_size`.
                    let per_iter_count = (total_count / sample_size as u128) as MaxCountUInt;

                    self.counters.push_counter(AnyCounter::known(counter_kind, per_iter_count));
                }

                if let Some(rem_samples) = &mut rem_samples {
                    *rem_samples = rem_samples.saturating_sub(1);
                }
            }

            if let Some(initial_start) = initial_start {
                let last_end = raw_samples.iter().map(|s| s.end).max().unwrap();
                elapsed_picos = last_end.duration_since(initial_start, timer).picos;
            } else {
                // Progress by at least 1ns to prevent extremely fast
                // functions from taking forever when `min_time` is set.
                let progress_picos = slowest_time.picos.max(1_000);
                elapsed_picos = elapsed_picos.saturating_add(progress_picos);
            }
        }

        // Reset flag for ignoring allocations.
        crate::alloc::IGNORE_ALLOC.set(false);
    }

    /// Returns a closure that takes the sample size and input counter, and then
    /// returns a newly recorded sample.
    fn sample_recorder<I, O>(
        &self,
        gen_input: impl Fn() -> I,
        benched: impl Fn(&UnsafeCell<MaybeUninit<I>>) -> O,
        drop_input: impl Fn(&UnsafeCell<MaybeUninit<I>>),
    ) -> impl Fn(usize, Option<&Barrier>, &mut dyn FnMut(&I)) -> ([Timestamp; 2], ThreadAllocInfo)
    {
        // We defer:
        // - Usage of `gen_input` values.
        // - Drop destructor for `O`, preventing it from affecting sample
        //   measurements. Outputs are stored into a pre-allocated buffer during
        //   the sample loop. The allocation is reused between samples to reduce
        //   time spent between samples.

        let timer_kind = self.shared_context.timer.kind();

        move |sample_size: usize, barrier: Option<&Barrier>, count_input: &mut dyn FnMut(&I)| {
            let mut defer_store = DeferStore::<I, O>::default();

            let mut saved_alloc_info = ThreadAllocInfo::new();
            let mut save_alloc_info = || {
                if crate::alloc::IGNORE_ALLOC.get() {
                    return;
                }

                if let Some(alloc_info) = ThreadAllocInfo::try_current() {
                    // SAFETY: We have exclusive access.
                    saved_alloc_info = unsafe { alloc_info.as_ptr().read() };
                }
            };

            // Synchronize all threads to start timed section simultaneously and
            // clear every thread's memory profiling info.
            //
            // This ensures work external to the timed section does not affect
            // the timing of other threads.
            let sync_threads = |is_start: bool| {
                sync_impl(barrier, is_start);

                // Monomorphize implementation to reduce code size.
                #[inline(never)]
                fn sync_impl(barrier: Option<&Barrier>, is_start: bool) {
                    // Ensure benchmarked section has a `ThreadAllocInfo`
                    // allocated for the current thread and clear previous info.
                    let alloc_info = if is_start { ThreadAllocInfo::current() } else { None };

                    // Synchronize all threads.
                    //
                    // This is the final synchronization point for the end.
                    if let Some(barrier) = barrier {
                        barrier.wait();
                    }

                    if let Some(mut alloc_info) = alloc_info {
                        // SAFETY: We have exclusive access.
                        let alloc_info = unsafe { alloc_info.as_mut() };

                        alloc_info.clear();

                        // Synchronize all threads.
                        if let Some(barrier) = barrier {
                            barrier.wait();
                        }
                    }
                }
            };

            // The following logic chooses how to efficiently sample the
            // benchmark function once and assigns `sample_start`/`sample_end`
            // before/after the sample loop.
            //
            // NOTE: Testing and benchmarking should behave exactly the same
            // when getting the sample time span. We don't want to introduce
            // extra work that may worsen measurement quality for real
            // benchmarking.
            let sample_start: UntaggedTimestamp;
            let sample_end: UntaggedTimestamp;

            if size_of::<I>() == 0 && (size_of::<O>() == 0 || !mem::needs_drop::<O>()) {
                // Use a range instead of `defer_store` to make the benchmarking
                // loop cheaper.

                // Run `gen_input` the expected number of times in case it
                // updates external state used by `benched`.
                for _ in 0..sample_size {
                    let input = gen_input();
                    count_input(&input);

                    // Inputs are consumed/dropped later.
                    mem::forget(input);
                }

                sync_threads(true);
                sample_start = UntaggedTimestamp::start(timer_kind);

                // Sample loop:
                for _ in 0..sample_size {
                    // SAFETY: Input is a ZST, so we can construct one out of
                    // thin air.
                    let input = unsafe { UnsafeCell::new(MaybeUninit::<I>::zeroed()) };

                    mem::forget(black_box(benched(&input)));
                }

                sample_end = UntaggedTimestamp::end(timer_kind);
                sync_threads(false);
                save_alloc_info();

                // Drop outputs and inputs.
                for _ in 0..sample_size {
                    // Output only needs drop if ZST.
                    if size_of::<O>() == 0 {
                        // SAFETY: Output is a ZST, so we can construct one out
                        // of thin air.
                        unsafe { _ = mem::zeroed::<O>() }
                    }

                    if mem::needs_drop::<I>() {
                        // SAFETY: Input is a ZST, so we can construct one out
                        // of thin air and not worry about aliasing.
                        unsafe { drop_input(&UnsafeCell::new(MaybeUninit::<I>::zeroed())) }
                    }
                }
            } else {
                defer_store.prepare(sample_size);

                match defer_store.slots() {
                    // Output needs to be dropped. We defer drop in the sample
                    // loop by inserting it into `defer_store`.
                    Ok(defer_slots_slice) => {
                        // Initialize and store inputs.
                        for DeferSlot { input, .. } in defer_slots_slice {
                            // SAFETY: We have exclusive access to `input`.
                            let input = unsafe { &mut *input.get() };
                            let input = input.write(gen_input());
                            count_input(input);

                            // Make input opaque to benchmarked function.
                            black_box(input);
                        }

                        // Create iterator before the sample timing section to
                        // reduce benchmarking overhead.
                        let defer_slots_iter = defer_slots_slice.iter();

                        sync_threads(true);
                        sample_start = UntaggedTimestamp::start(timer_kind);

                        // Sample loop:
                        for defer_slot in defer_slots_iter {
                            // SAFETY: All inputs in `defer_store` were
                            // initialized and we have exclusive access to the
                            // output slot.
                            unsafe {
                                let output = benched(&defer_slot.input);
                                *defer_slot.output.get() = MaybeUninit::new(output);
                            }
                        }

                        sample_end = UntaggedTimestamp::end(timer_kind);
                        sync_threads(false);
                        save_alloc_info();

                        // Prevent the optimizer from removing writes to inputs
                        // and outputs in the sample loop.
                        black_box(defer_slots_slice);

                        // Drop outputs and inputs.
                        for DeferSlot { input, output } in defer_slots_slice {
                            // SAFETY: All outputs were initialized in the
                            // sample loop and we have exclusive access.
                            unsafe { (*output.get()).assume_init_drop() }

                            if mem::needs_drop::<I>() {
                                // SAFETY: The output was dropped and thus we
                                // have exclusive access to inputs.
                                unsafe { drop_input(input) }
                            }
                        }
                    }

                    // Output does not need to be dropped.
                    Err(defer_inputs_slice) => {
                        // Initialize and store inputs.
                        for input in defer_inputs_slice {
                            // SAFETY: We have exclusive access to `input`.
                            let input = unsafe { &mut *input.get() };
                            let input = input.write(gen_input());
                            count_input(input);

                            // Make input opaque to benchmarked function.
                            black_box(input);
                        }

                        // Create iterator before the sample timing section to
                        // reduce benchmarking overhead.
                        let defer_inputs_iter = defer_inputs_slice.iter();

                        sync_threads(true);
                        sample_start = UntaggedTimestamp::start(timer_kind);

                        // Sample loop:
                        for input in defer_inputs_iter {
                            // SAFETY: All inputs in `defer_store` were
                            // initialized.
                            black_box_drop(unsafe { benched(input) });
                        }

                        sample_end = UntaggedTimestamp::end(timer_kind);
                        sync_threads(false);
                        save_alloc_info();

                        // Prevent the optimizer from removing writes to inputs
                        // in the sample loop.
                        black_box(defer_inputs_slice);

                        // Drop inputs.
                        if mem::needs_drop::<I>() {
                            for input in defer_inputs_slice {
                                // SAFETY: We have exclusive access to inputs.
                                unsafe { drop_input(input) }
                            }
                        }
                    }
                }
            }

            // SAFETY: These values are guaranteed to be the correct variant
            // because they were created from the same `timer_kind`.
            let interval = unsafe {
                [sample_start.into_timestamp(timer_kind), sample_end.into_timestamp(timer_kind)]
            };

            (interval, saved_alloc_info)
        }
    }

    #[inline]
    fn initial_mode(&self) -> BenchMode {
        if self.shared_context.action.is_test() {
            BenchMode::Test
        } else if let Some(sample_size) = self.options.sample_size {
            BenchMode::Collect { sample_size }
        } else {
            BenchMode::Tune { sample_size: 1 }
        }
    }

    pub fn compute_stats(&self) -> Stats {
        let time_samples = &self.samples.time_samples;
        let alloc_info_by_sample = &self.samples.alloc_info_by_sample;

        let sample_count = time_samples.len();
        let sample_size = self.samples.sample_size;

        let total_count = self.samples.iter_count();

        let total_duration = self.samples.total_duration();
        let mean_duration = FineDuration {
            picos: total_duration.picos.checked_div(total_count as u128).unwrap_or_default(),
        };

        // Samples sorted by duration.
        let sorted_samples = self.samples.sorted_samples();
        let median_samples = util::slice_middle(&sorted_samples);

        let index_of_sample = |sample: &TimeSample| -> usize {
            util::slice_ptr_index(&self.samples.time_samples, sample)
        };

        let counter_count_for_sample =
            |sample: &TimeSample, counter_kind: KnownCounterKind| -> Option<MaxCountUInt> {
                let counts = self.counters.counts(counter_kind);

                let index = if self.counters.uses_input_counts(counter_kind) {
                    index_of_sample(sample)
                } else {
                    0
                };

                counts.get(index).copied()
            };

        let min_duration =
            sorted_samples.first().map(|s| s.duration / sample_size).unwrap_or_default();
        let max_duration =
            sorted_samples.last().map(|s| s.duration / sample_size).unwrap_or_default();

        let median_duration = if median_samples.is_empty() {
            FineDuration::default()
        } else {
            let sum: u128 = median_samples.iter().map(|s| s.duration.picos).sum();
            FineDuration { picos: sum / median_samples.len() as u128 } / sample_size
        };

        let counts = KnownCounterKind::ALL.map(|counter_kind| {
            let median: MaxCountUInt = {
                let mut sum: u128 = 0;

                for sample in median_samples {
                    let sample_count = counter_count_for_sample(sample, counter_kind)? as u128;

                    // Saturating add in case `MaxUIntCount > u64`.
                    sum = sum.saturating_add(sample_count);
                }

                (sum / median_samples.len() as u128) as MaxCountUInt
            };

            Some(StatsSet {
                fastest: sorted_samples
                    .first()
                    .and_then(|s| counter_count_for_sample(s, counter_kind))?,
                slowest: sorted_samples
                    .last()
                    .and_then(|s| counter_count_for_sample(s, counter_kind))?,
                median,
                mean: self.counters.mean_count(counter_kind),
            })
        });

        let sample_alloc_info = |sample: Option<&TimeSample>| -> Option<&ThreadAllocInfo> {
            sample
                .and_then(|sample| u32::try_from(index_of_sample(sample)).ok())
                .and_then(|index| self.samples.alloc_info_by_sample.get(&index))
        };

        let sample_alloc_tally = |sample: Option<&TimeSample>, op: AllocOp| -> ThreadAllocTally {
            sample_alloc_info(sample)
                .map(|alloc_info| alloc_info.tallies.get(op))
                .copied()
                .unwrap_or_default()
        };

        let mut alloc_total_max_count = 0u128;
        let mut alloc_total_max_size = 0u128;
        let mut alloc_total_tallies = TotalAllocTallyMap::default();

        for alloc_info in alloc_info_by_sample.values() {
            alloc_total_max_count += alloc_info.max_count as u128;
            alloc_total_max_size += alloc_info.max_size as u128;
            alloc_info.tallies.add_to_total(&mut alloc_total_tallies);
        }

        let sample_size = f64::from(sample_size);
        Stats {
            sample_count: sample_count as u32,
            iter_count: total_count,
            time: StatsSet {
                fastest: min_duration,
                slowest: max_duration,
                median: median_duration,
                mean: mean_duration,
            },
            max_alloc: StatsSet {
                fastest: {
                    let alloc_info = sample_alloc_info(sorted_samples.first().copied());

                    AllocTally {
                        count: alloc_info.map(|info| info.max_count as f64).unwrap_or_default()
                            / sample_size,
                        size: alloc_info.map(|info| info.max_size as f64).unwrap_or_default()
                            / sample_size,
                    }
                },
                slowest: {
                    let alloc_info = sample_alloc_info(sorted_samples.last().copied());

                    AllocTally {
                        count: alloc_info.map(|info| info.max_count as f64).unwrap_or_default()
                            / sample_size,
                        size: alloc_info.map(|info| info.max_size as f64).unwrap_or_default()
                            / sample_size,
                    }
                },
                // TODO: Switch to median of alloc info itself, rather than
                // basing off of median times.
                median: {
                    let alloc_info_for_median =
                        |index| sample_alloc_info(median_samples.get(index).copied());

                    let max_count_for_median = |index: usize| -> f64 {
                        alloc_info_for_median(index)
                            .map(|info| info.max_count as f64)
                            .unwrap_or_default()
                    };

                    let max_size_for_median = |index: usize| -> f64 {
                        alloc_info_for_median(index)
                            .map(|info| info.max_size as f64)
                            .unwrap_or_default()
                    };

                    let median_count = median_samples.len().max(1) as f64;

                    let median_max_count = max_count_for_median(0) + max_count_for_median(1);
                    let median_max_size = max_size_for_median(0) + max_size_for_median(1);

                    AllocTally {
                        count: median_max_count / median_count / sample_size,
                        size: median_max_size / median_count / sample_size,
                    }
                },
                mean: AllocTally {
                    count: alloc_total_max_count as f64 / total_count as f64,
                    size: alloc_total_max_size as f64 / total_count as f64,
                },
            }
            .transpose(),
            alloc_tallies: AllocOpMap {
                values: AllocOp::ALL
                    .map(|op| StatsSet {
                        fastest: {
                            let fastest = sample_alloc_tally(sorted_samples.first().copied(), op);

                            AllocTally {
                                count: fastest.count as f64 / sample_size,
                                size: fastest.size as f64 / sample_size,
                            }
                        },
                        slowest: {
                            let slowest = sample_alloc_tally(sorted_samples.last().copied(), op);

                            AllocTally {
                                count: slowest.count as f64 / sample_size,
                                size: slowest.size as f64 / sample_size,
                            }
                        },
                        median: {
                            let tally_for_median = |index: usize| -> ThreadAllocTally {
                                sample_alloc_tally(median_samples.get(index).copied(), op)
                            };

                            let a = tally_for_median(0);
                            let b = tally_for_median(1);

                            let median_count = median_samples.len().max(1) as f64;

                            let avg_count = (a.count as f64 + b.count as f64) / median_count;
                            let avg_size = (a.size as f64 + b.size as f64) / median_count;

                            AllocTally {
                                count: avg_count / sample_size,
                                size: avg_size / sample_size,
                            }
                        },
                        mean: {
                            let tally = alloc_total_tallies.get(op);
                            AllocTally {
                                count: tally.count as f64 / total_count as f64,
                                size: tally.size as f64 / total_count as f64,
                            }
                        },
                    })
                    .map(StatsSet::transpose),
            },
            counts,
        }
    }
}

impl<T> StatsSet<AllocTally<T>> {
    #[inline]
    pub fn transpose(self) -> AllocTally<StatsSet<T>> {
        AllocTally {
            count: StatsSet {
                fastest: self.fastest.count,
                slowest: self.slowest.count,
                median: self.median.count,
                mean: self.mean.count,
            },
            size: StatsSet {
                fastest: self.fastest.size,
                slowest: self.slowest.size,
                median: self.median.size,
                mean: self.mean.size,
            },
        }
    }
}
