//! [bench_attr]: macro@bench
//! [bench_attr_examples]: macro@bench#examples
//! [bench_attr_threads]: macro@bench#threads
#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![allow(
    unknown_lints,
    unused_unsafe,
    clippy::needless_doctest_main,
    clippy::needless_lifetimes,
    clippy::new_without_default,
    clippy::type_complexity,
    clippy::missing_transmute_annotations
)]

// Used by generated code. Not public API and thus not subject to SemVer.
#[doc(hidden)]
#[path = "private.rs"]
pub mod __private;

mod alloc;
mod bench;
mod cli;
mod compile_fail;
mod config;
mod divan;
mod entry;
mod stats;
mod thread_pool;
mod time;
mod tree_painter;
mod util;

pub mod counter;

/// Prevents compiler optimizations on a value.
///
/// `black_box` should only be used on [inputs](#benchmark-inputs) and
/// [outputs](#benchmark-outputs) of benchmarks. Newcomers to benchmarking may
/// be tempted to also use `black_box` within the implementation, but doing so
/// will overly pessimize the measured code without any benefit.
///
/// ## Benchmark Inputs
///
/// When benchmarking, it's good practice to ensure measurements are accurate by
/// preventing the compiler from optimizing based on assumptions about benchmark
/// inputs.
///
/// The compiler can optimize code for indices it knows about, such as by
/// removing bounds checks or unrolling loops. If real-world use of your code
/// would not know indices up front, consider preventing optimizations on them
/// in benchmarks:
///
/// ```
/// use divan::black_box;
///
/// const INDEX: usize = // ...
/// # 0;
/// const SLICE: &[u8] = // ...
/// # &[];
///
/// #[divan::bench]
/// fn bench() {
///     # fn work<T>(_: T) {}
///     work(&SLICE[black_box(INDEX)..]);
/// }
/// ```
///
/// The compiler may also optimize for the data itself, which can also be
/// avoided with `black_box`:
///
/// ```
/// # use divan::black_box;
/// # const INDEX: usize = 0;
/// # const SLICE: &[u8] = &[];
/// #[divan::bench]
/// fn bench() {
///     # fn work<T>(_: T) {}
///     work(black_box(&SLICE[black_box(INDEX)..]));
/// }
/// ```
///
/// ## Benchmark Outputs
///
/// When benchmarking, it's best to ensure that all of the code is actually
/// being run. If the compiler knows an output is unused, it may remove the code
/// that generated the output. This optimization can make benchmarks appear much
/// faster than they really are.
///
/// At the end of a benchmark, we can force the compiler to treat outputs as if
/// they were actually used:
///
/// ```
/// # use divan::black_box;
/// #[divan::bench]
/// fn bench() {
///     # let value = 1;
///     black_box(value.to_string());
/// }
/// ```
///
/// To make the code clearer to readers that the output is discarded, this code
/// could instead call [`black_box_drop`].
///
/// Alternatively, the output can be returned from the benchmark:
///
/// ```
/// #[divan::bench]
/// fn bench() -> String {
///     # let value = 1;
///     value.to_string()
/// }
/// ```
///
/// Returning the output will `black_box` it and also avoid measuring the time
/// to [drop](Drop) the output, which in this case is the time to deallocate a
/// [`String`]. Read more about this in the [`#[divan::bench]`
/// docs](macro@bench#drop).
///
/// ---
///
/// <h1>Standard Library Documentation</h1>
///
#[doc(inline)]
pub use std::hint::black_box;

#[doc(inline)]
pub use crate::{alloc::AllocProfiler, bench::Bencher, divan::Divan};

/// Runs all registered benchmarks.
///
/// # Examples
///
/// ```
/// #[divan::bench]
/// fn add() -> i32 {
///     // ...
///     # 0
/// }
///
/// fn main() {
///     // Run `add` benchmark:
///     divan::main();
/// }
/// ```
///
/// See [`#[divan::bench]`](macro@bench) for more examples.
pub fn main() {
    Divan::from_args().main();
}

/// [`black_box`] + [`drop`] convenience function.
///
/// # Examples
///
/// This is useful when benchmarking a lazy [`Iterator`] to completion with
/// [`for_each`](Iterator::for_each):
///
/// ```
/// #[divan::bench]
/// fn parse_iter() {
///     let input: &str = // ...
///     # "";
///
///     # struct Parser;
///     # impl Parser {
///     #   fn new(_: &str) -> Parser { Parser }
///     #   fn for_each(self, _: fn(&'static str)) {}
///     # }
///     Parser::new(input)
///         .for_each(divan::black_box_drop);
/// }
/// ```
#[inline]
pub fn black_box_drop<T>(dummy: T) {
    _ = black_box(dummy);
}

/// Registers a benchmarking function.
///
/// # Examples
///
/// The quickest way to get started is to benchmark the function as-is:
///
/// ```
/// use divan::black_box;
///
/// #[divan::bench]
/// fn add() -> i32 {
///     black_box(1) + black_box(42)
/// }
///
/// fn main() {
///     // Run `add` benchmark:
///     divan::main();
/// }
/// ```
///
/// If benchmarks need to setup context before running, they can take a
/// [`Bencher`] and use [`Bencher::bench`]:
///
/// ```
/// use divan::{Bencher, black_box};
///
/// #[divan::bench]
/// fn copy_from_slice(bencher: Bencher) {
///     let src = (0..100).collect::<Vec<i32>>();
///     let mut dst = vec![0; src.len()];
///
///     bencher.bench_local(move || {
///         black_box(&mut dst).copy_from_slice(black_box(&src));
///     });
/// }
/// ```
///
/// Applying this attribute multiple times to the same item will cause a compile
/// error:
///
/// ```compile_fail
/// #[divan::bench]
/// #[divan::bench]
/// fn bench() {
///     // ...
/// }
/// ```
///
/// # Drop
///
/// When a benchmarked function returns a value, it will not be [dropped][Drop]
/// until after the current sample loop is finished. This allows for more
/// precise timing measurements.
///
/// Note that there is an inherent memory cost to defer drop, including
/// allocations inside not-yet-dropped values. Also, if the benchmark
/// [panics](macro@std::panic), the values will never be dropped.
///
/// The following example benchmarks will only measure [`String`] construction
/// time, but not deallocation time:
///
/// ```
/// use divan::{Bencher, black_box};
///
/// #[divan::bench]
/// fn freestanding() -> String {
///     black_box("hello").to_uppercase()
/// }
///
/// #[divan::bench]
/// fn contextual(bencher: Bencher) {
///     // Setup:
///     let s: String = // ...
///     # String::new();
///
///     bencher.bench(|| -> String {
///         black_box(&s).to_lowercase()
///     });
/// }
/// ```
///
/// If the returned value *does not* need to be dropped, there is no memory
/// cost. Because of this, the following example benchmarks are equivalent:
///
/// ```
/// #[divan::bench]
/// fn with_return() -> i32 {
///     let n: i32 = // ...
///     # 0;
///     n
/// }
///
/// #[divan::bench]
/// fn without_return() {
///     let n: i32 = // ...
///     # 0;
///     divan::black_box(n);
/// }
/// ```
///
/// # Options
///
/// - [`name`]
/// - [`crate`]
/// - [`args`]
/// - [`consts`]
/// - [`types`]
/// - [`sample_count`]
/// - [`sample_size`]
/// - [`threads`]
/// - [`counters`]
///     - [`bytes_count`]
///     - [`chars_count`]
///     - [`items_count`]
/// - [`min_time`]
/// - [`max_time`]
/// - [`skip_ext_time`]
/// - [`ignore`]
///
/// ## `name`
/// [`name`]: #name
///
/// By default, the benchmark uses the function's name. It can be overridden via
/// the [`name`] option:
///
/// ```
/// #[divan::bench(name = "my_add")]
/// fn add() -> i32 {
///     // Will appear as "crate_name::my_add".
///     # 0
/// }
/// ```
///
/// ## `crate`
/// [`crate`]: #crate
///
/// The path to the specific `divan` crate instance used by this macro's
/// generated code can be specified via the [`crate`] option. This is applicable
/// when using `divan` via a macro from your own crate.
///
/// ```
/// extern crate divan as sofa;
///
/// #[::sofa::bench(crate = ::sofa)]
/// fn add() -> i32 {
///     // ...
///     # 0
/// }
/// ```
///
/// ## `args`
/// [`args`]: #args
///
/// Function arguments can be provided to benchmark the function over multiple
/// cases. This is used for comparing across parameters like collection lengths
/// and [`enum`](https://doc.rust-lang.org/std/keyword.enum.html) variants. If
/// you are not comparing cases and just need to pass a value into the
/// benchmark, instead consider passing local values into the [`Bencher::bench`]
/// closure or use [`Bencher::with_inputs`] for many distinct values.
///
/// The following example benchmarks converting a [`Range`](std::ops::Range) to
/// [`Vec`] over different lengths:
///
/// ```
/// #[divan::bench(args = [1000, LEN, len()])]
/// fn init_vec(len: usize) -> Vec<usize> {
///     (0..len).collect()
/// }
///
/// const LEN: usize = // ...
/// # 0;
///
/// fn len() -> usize {
///     // ...
///     # 0
/// }
/// ```
///
/// The list of arguments can be shared across multiple benchmarks through an
/// external [`Iterator`]:
///
/// ```
/// const LENS: &[usize] = // ...
/// # &[];
///
/// #[divan::bench(args = LENS)]
/// fn bench_vec1(len: usize) -> Vec<usize> {
///     // ...
///     # vec![]
/// }
///
/// #[divan::bench(args = LENS)]
/// fn bench_vec2(len: usize) -> Vec<usize> {
///     // ...
///     # vec![]
/// }
/// ```
///
/// Unlike the [`consts`] option, any argument type is supported if it
/// implements [`Any`], [`Copy`], [`Send`], [`Sync`], and [`ToString`] (or
/// [`Debug`](std::fmt::Debug)):
///
/// ```
/// #[derive(Clone, Copy, Debug)]
/// enum Arg {
///     A, B
/// }
///
/// #[divan::bench(args = [Arg::A, Arg::B])]
/// fn bench_args(arg: Arg) {
///     // ...
/// }
/// ```
///
/// The argument type does not need to implement [`Copy`] if it is used through
/// a reference:
///
/// ```
/// #[derive(Debug)]
/// enum Arg {
///     A, B
/// }
///
/// #[divan::bench(args = [Arg::A, Arg::B])]
/// fn bench_args(arg: &Arg) {
///     // ...
/// }
/// ```
///
/// For convenience, common string types are coerced to [`&str`](primitive@str):
///
/// ```
/// fn strings() -> impl Iterator<Item = String> {
///     // ...
///     # [].into_iter()
/// }
///
/// #[divan::bench(args = strings())]
/// fn bench_strings(s: &str) {
///     // ...
/// }
/// ```
///
/// Arguments can also be used with [`Bencher`]. This allows for generating
/// inputs based on [`args`] values or providing throughput information via
/// [`Counter`s](crate::counter::Counter):
///
/// ```
/// # fn new_value<T>(v: T) -> T { v }
/// # fn do_work<T>(_: T) {}
/// use divan::Bencher;
///
/// #[divan::bench(args = [1, 2, 3])]
/// fn bench(bencher: Bencher, len: usize) {
///     let value = new_value(len);
///
///     bencher
///         .counter(len)
///         .bench(|| {
///             do_work(value);
///         });
/// }
/// ```
///
/// ## `consts`
/// [`consts`]: #consts
///
/// Divan supports benchmarking functions with [`const`
/// generics](https://doc.rust-lang.org/reference/items/generics.html#const-generics)
/// via the [`consts`] option.
///
/// The following example benchmarks initialization of [`[i32; N]`](prim@array)
/// for values of `N` provided by a [literal](https://doc.rust-lang.org/reference/expressions/literal-expr.html),
/// [`const` item](https://doc.rust-lang.org/reference/items/constant-items.html),
/// and [`const fn`](https://doc.rust-lang.org/reference/const_eval.html#const-functions):
///
/// ```
/// #[divan::bench(consts = [1000, LEN, len()])]
/// fn init_array<const N: usize>() -> [i32; N] {
///     let mut result = [0; N];
///
///     for i in 0..N {
///         result[i] = divan::black_box(i as i32);
///     }
///
///     result
/// }
///
/// const LEN: usize = // ...
/// # 0;
///
/// const fn len() -> usize {
///     // ...
///     # 0
/// }
/// ```
///
/// The list of constants can be shared across multiple benchmarks through an
/// external [array](prim@array) or [slice](prim@slice):
///
/// ```
/// const SIZES: &[usize] = &[1, 2, 5, 10];
///
/// #[divan::bench(consts = SIZES)]
/// fn bench_array1<const N: usize>() -> [i32; N] {
///     // ...
///     # [0; N]
/// }
///
/// #[divan::bench(consts = SIZES)]
/// fn bench_array2<const N: usize>() -> [i32; N] {
///     // ...
///     # [0; N]
/// }
/// ```
///
/// External constants are limited to lengths 1 through 20, because of
/// implementation details. This limit does not apply if the list is provided
/// directly like in the first example.
///
/// ```compile_fail
/// const SIZES: [usize; 21] = [
///     // ...
///     # 0; 21
/// ];
///
/// #[divan::bench(consts = SIZES)]
/// fn bench_array<const N: usize>() -> [i32; N] {
///     // ...
///     # [0; N]
/// }
/// ```
///
/// ## `types`
/// [`types`]: #types
///
/// Divan supports benchmarking generic functions over a list of types via the
/// [`types`] option.
///
/// The following example benchmarks the [`From<&str>`](From) implementations
/// for [`&str`](prim@str) and [`String`]:
///
/// ```
/// #[divan::bench(types = [&str, String])]
/// fn from_str<'a, T>() -> T
/// where
///     T: From<&'a str>,
/// {
///     divan::black_box("hello world").into()
/// }
/// ```
///
/// The [`types`] and [`args`] options can be combined to benchmark _T_ × _A_
/// scenarios. The following example benchmarks the [`FromIterator`]
/// implementations for [`Vec`], [`BTreeSet`], and [`HashSet`]:
///
/// ```
/// use std::collections::{BTreeSet, HashSet};
///
/// #[divan::bench(
///     types = [Vec<i32>, BTreeSet<i32>, HashSet<i32>],
///     args = [0, 2, 4, 16, 256, 4096],
/// )]
/// fn from_range<T>(n: i32) -> T
/// where
///     T: FromIterator<i32>,
/// {
///     (0..n).collect()
/// }
/// ```
///
/// [`BTreeSet`]: std::collections::BTreeSet
/// [`HashSet`]: std::collections::HashSet
///
/// ## `sample_count`
/// [`sample_count`]: #sample_count
///
/// The number of statistical sample recordings can be set to a predetermined
/// [`u32`] value via the [`sample_count`] option. This may be overridden at
/// runtime using either the `DIVAN_SAMPLE_COUNT` environment variable or
/// `--sample-count` CLI argument.
///
/// ```
/// #[divan::bench(sample_count = 1000)]
/// fn add() -> i32 {
///     // ...
///     # 0
/// }
/// ```
///
/// If the [`threads`] option is enabled, sample count becomes a multiple of the
/// number of threads. This is because each thread operates over the same sample
/// size to ensure there are always N competing threads doing the same amount of
/// work.
///
/// ## `sample_size`
/// [`sample_size`]: #sample_size
///
/// The number iterations within each statistics sample can be set to a
/// predetermined [`u32`] value via the [`sample_size`] option. This may be
/// overridden at runtime using either the `DIVAN_SAMPLE_SIZE` environment
/// variable or `--sample-size` CLI argument.
///
/// ```
/// #[divan::bench(sample_size = 1000)]
/// fn add() -> i32 {
///     // ...
///     # 0
/// }
/// ```
///
/// ## `threads`
/// [`threads`]: #threads
///
/// Benchmarked functions can be run across multiple threads via the [`threads`]
/// option. This enables you to measure contention on [atomics and
/// locks][std::sync]. The default thread count is the [available parallelism].
///
/// ```
/// use std::sync::Arc;
///
/// #[divan::bench(threads)]
/// fn arc_clone(bencher: divan::Bencher) {
///     let arc = Arc::new(42);
///
///     bencher.bench(|| arc.clone());
/// }
/// ```
///
/// The [`threads`] option can be set to any of:
/// - [`bool`] for [available parallelism] (true) or no parallelism.
/// - [`usize`] for a specific number of threads. 0 means use [available
///   parallelism] and 1 means no parallelism.
/// - [`IntoIterator`] over [`usize`] for multiple thread counts, such as:
///     - [`Range<usize>`](std::ops::Range)
///     - [`[usize; N]`](prim@array)
///     - [`&[usize]`](prim@slice)
///
/// ```
/// #[divan::bench(threads = false)]
/// fn single() {
///     // ...
/// }
///
/// #[divan::bench(threads = 10)]
/// fn specific() {
///     // ...
/// }
///
/// #[divan::bench(threads = 0..=8)]
/// fn range() {
///     // Note: Includes 0 for available parallelism.
/// }
///
/// #[divan::bench(threads = [0, 1, 4, 8, 16])]
/// fn selection() {
///     // ...
/// }
/// ```
///
/// ## `counters`
/// [`counters`]: #counters
///
/// The [`Counter`s](crate::counter::Counter) of each iteration can be set via
/// the [`counters`] option. The following example emits info for the number of
/// bytes and number of ints processed when benchmarking [slice sorting](slice::sort):
///
/// ```
/// use divan::{Bencher, counter::{BytesCount, ItemsCount}};
///
/// const INTS: &[i32] = &[
///     // ...
/// ];
///
/// #[divan::bench(counters = [
///     BytesCount::of_slice(INTS),
///     ItemsCount::new(INTS.len()),
/// ])]
/// fn sort(bencher: Bencher) {
///     bencher
///         .with_inputs(|| INTS.to_vec())
///         .bench_refs(|ints| ints.sort());
/// }
/// ```
///
/// For convenience, singular `counter` allows a single
/// [`Counter`](crate::counter::Counter) to be set. The following example emits
/// info for the number of bytes processed when benchmarking
/// [`char`-counting](std::str::Chars::count):
///
/// ```
/// use divan::counter::BytesCount;
///
/// const STR: &str = "...";
///
/// #[divan::bench(counter = BytesCount::of_str(STR))]
/// fn char_count() -> usize {
///     divan::black_box(STR).chars().count()
/// }
/// ```
///
/// See:
/// - [`#[divan::bench_group(counters = ...)]`](macro@bench_group#counters)
/// - [`Bencher::counter`]
/// - [`Bencher::input_counter`]
///
/// ### `bytes_count`
/// [`bytes_count`]: #bytes_count
///
/// Convenience shorthand for
/// <code>[counter](#counters) = [BytesCount](counter::BytesCount)::from(n)</code>.
///
/// ### `chars_count`
/// [`chars_count`]: #chars_count
///
/// Convenience shorthand for
/// <code>[counter](#counters) = [CharsCount](counter::CharsCount)::from(n)</code>.
///
/// ### `items_count`
/// [`items_count`]: #items_count
///
/// Convenience shorthand for
/// <code>[counter](#counters) = [ItemsCount](counter::ItemsCount)::from(n)</code>.
///
/// ## `min_time`
/// [`min_time`]: #min_time
///
/// The minimum time spent benchmarking each function can be set to a
/// predetermined [`Duration`] via the [`min_time`] option. This may be
/// overridden at runtime using either the `DIVAN_MIN_TIME` environment variable
/// or `--min-time` CLI argument.
///
/// Unless [`skip_ext_time`] is set, this includes time external to the
/// benchmarked function, such as time spent generating inputs and running
/// [`Drop`].
///
/// ```
/// use std::time::Duration;
///
/// #[divan::bench(min_time = Duration::from_secs(3))]
/// fn add() -> i32 {
///     // ...
///     # 0
/// }
/// ```
///
/// For convenience, [`min_time`] can also be set with seconds as [`u64`] or
/// [`f64`]. Invalid values will cause a panic at runtime.
///
/// ```
/// #[divan::bench(min_time = 2)]
/// fn int_secs() -> i32 {
///     // ...
///     # 0
/// }
///
/// #[divan::bench(min_time = 1.5)]
/// fn float_secs() -> i32 {
///     // ...
///     # 0
/// }
/// ```
///
/// ## `max_time`
/// [`max_time`]: #max_time
///
/// The maximum time spent benchmarking each function can be set to a
/// predetermined [`Duration`] via the [`max_time`] option. This may be
/// overridden at runtime using either the `DIVAN_MAX_TIME` environment variable
/// or `--max-time` CLI argument.
///
/// Unless [`skip_ext_time`] is set, this includes time external to the
/// benchmarked function, such as time spent generating inputs and running
/// [`Drop`].
///
/// If `min_time > max_time`, then [`max_time`] has priority and [`min_time`]
/// will not be reached.
///
/// ```
/// use std::time::Duration;
///
/// #[divan::bench(max_time = Duration::from_secs(5))]
/// fn add() -> i32 {
///     // ...
///     # 0
/// }
/// ```
///
/// For convenience, like [`min_time`], [`max_time`] can also be set with
/// seconds as [`u64`] or [`f64`]. Invalid values will cause a panic at runtime.
///
/// ```
/// #[divan::bench(max_time = 8)]
/// fn int_secs() -> i32 {
///     // ...
///     # 0
/// }
///
/// #[divan::bench(max_time = 9.5)]
/// fn float_secs() -> i32 {
///     // ...
///     # 0
/// }
/// ```
///
/// ## `skip_ext_time`
/// [`skip_ext_time`]: #skip_ext_time
///
/// By default, [`min_time`] and [`max_time`] include time external to the
/// benchmarked function, such as time spent generating inputs and running
/// [`Drop`]. Enabling the [`skip_ext_time`] option will instead make those
/// options only consider time spent within the benchmarked function. This may
/// be overridden at runtime using either the `DIVAN_SKIP_EXT_TIME` environment
/// variable or `--skip-ext-time` CLI argument.
///
/// In the following example, [`max_time`] only considers time spent running
/// `measured_function`:
///
/// ```
/// # fn generate_input() {}
/// # fn measured_function(_: ()) {}
/// #[divan::bench(max_time = 5, skip_ext_time)]
/// fn bench(bencher: divan::Bencher) {
///     bencher
///         .with_inputs(|| generate_input())
///         .bench_values(|input| measured_function(input));
/// }
/// ```
///
/// This option can be set to an explicit [`bool`] value to override parent
/// values:
///
/// ```
/// #[divan::bench(max_time = 5, skip_ext_time = false)]
/// fn bench(bencher: divan::Bencher) {
///     // ...
/// }
/// ```
///
/// ## `ignore`
/// [`ignore`]: #ignore
///
/// Like [`#[test]`](https://doc.rust-lang.org/reference/attributes/testing.html#the-test-attribute),
/// `#[divan::bench]` functions can use [`#[ignore]`](https://doc.rust-lang.org/reference/attributes/testing.html#the-ignore-attribute):
///
/// ```
/// #[divan::bench]
/// #[ignore]
/// fn todo() {
///     unimplemented!();
/// }
/// # divan::main();
/// ```
///
/// This option can also instead be set within the `#[divan::bench]` attribute:
///
/// ```
/// #[divan::bench(ignore)]
/// fn todo() {
///     unimplemented!();
/// }
/// # divan::main();
/// ```
///
/// Like [`skip_ext_time`], this option can be set to an explicit [`bool`] value
/// to override parent values:
///
/// ```
/// #[divan::bench(ignore = false)]
/// fn bench() {
///     // ...
/// }
/// ```
///
/// This can be used to ignore benchmarks based on a runtime condition. The
/// following example benchmark will be ignored if an [environment
/// variable](std::env::var) is not set to "true":
///
/// ```
/// #[divan::bench(
///     ignore = std::env::var("BENCH_EXPENSIVE").as_deref() != Ok("true")
/// )]
/// fn expensive_bench() {
///     // ...
/// }
/// ```
///
/// [`Any`]: std::any::Any
/// [`Duration`]: std::time::Duration
/// [available parallelism]: std::thread::available_parallelism
pub use divan_macros::bench;

/// Registers a benchmarking group.
///
/// # Examples
///
/// This is used for setting [options] shared across
/// [`#[divan::bench]`](macro@bench) functions in the same module:
///
/// ```
/// #[divan::bench_group(
///     sample_count = 100,
///     sample_size = 500,
/// )]
/// mod math {
///     use divan::black_box;
///
///     #[divan::bench]
///     fn add() -> i32 {
///         black_box(1) + black_box(42)
///     }
///
///     #[divan::bench]
///     fn div() -> i32 {
///         black_box(1) / black_box(42)
///     }
/// }
///
/// fn main() {
///     // Run `math::add` and `math::div` benchmarks:
///     divan::main();
/// }
/// ```
///
/// Benchmarking [options] set on parent groups cascade into child groups and
/// their benchmarks:
///
/// ```
/// #[divan::bench_group(
///     sample_count = 100,
///     sample_size = 500,
/// )]
/// mod parent {
///     #[divan::bench_group(sample_size = 1)]
///     mod child1 {
///         #[divan::bench]
///         fn bench() {
///             // Will be sampled 100 times with 1 iteration per sample.
///         }
///     }
///
///     #[divan::bench_group(sample_count = 42)]
///     mod child2 {
///         #[divan::bench]
///         fn bench() {
///             // Will be sampled 42 times with 500 iterations per sample.
///         }
///     }
///
///     mod child3 {
///         #[divan::bench(sample_count = 1)]
///         fn bench() {
///             // Will be sampled 1 time with 500 iterations per sample.
///         }
///     }
/// }
/// ```
///
/// Applying this attribute multiple times to the same item will cause a compile
/// error:
///
/// ```compile_fail
/// #[divan::bench_group]
/// #[divan::bench_group]
/// mod math {
///     // ...
/// }
/// ```
///
/// # Options
/// [options]: #options
///
/// - [`name`]
/// - [`crate`]
/// - [`sample_count`]
/// - [`sample_size`]
/// - [`threads`]
/// - [`counters`]
///     - [`bytes_count`]
///     - [`chars_count`]
///     - [`items_count`]
/// - [`min_time`]
/// - [`max_time`]
/// - [`skip_ext_time`]
/// - [`ignore`]
///
/// ## `name`
/// [`name`]: #name
///
/// By default, the benchmark group uses the module's name. It can be overridden
/// via the `name` option:
///
/// ```
/// #[divan::bench_group(name = "my_math")]
/// mod math {
///     #[divan::bench(name = "my_add")]
///     fn add() -> i32 {
///         // Will appear as "crate_name::my_math::my_add".
///         # 0
///     }
/// }
/// ```
///
/// ## `crate`
/// [`crate`]: #crate
///
/// The path to the specific `divan` crate instance used by this macro's
/// generated code can be specified via the [`crate`] option. This is applicable
/// when using `divan` via a macro from your own crate.
///
/// ```
/// extern crate divan as sofa;
///
/// #[::sofa::bench_group(crate = ::sofa)]
/// mod math {
///     #[::sofa::bench(crate = ::sofa)]
///     fn add() -> i32 {
///         // ...
///         # 0
///     }
/// }
/// ```
///
/// ## `sample_count`
/// [`sample_count`]: #sample_count
///
/// The number of statistical sample recordings can be set to a predetermined
/// [`u32`] value via the [`sample_count`] option. This may be overridden at
/// runtime using either the `DIVAN_SAMPLE_COUNT` environment variable or
/// `--sample-count` CLI argument.
///
/// ```
/// #[divan::bench_group(sample_count = 1000)]
/// mod math {
///     #[divan::bench]
///     fn add() -> i32 {
///         // ...
///         # 0
///     }
/// }
/// ```
///
/// If the [`threads`] option is enabled, sample count becomes a multiple of the
/// number of threads. This is because each thread operates over the same sample
/// size to ensure there are always N competing threads doing the same amount of
/// work.
///
/// ## `sample_size`
/// [`sample_size`]: #sample_size
///
/// The number iterations within each statistical sample can be set to a
/// predetermined [`u32`] value via the [`sample_size`] option. This may be
/// overridden at runtime using either the `DIVAN_SAMPLE_SIZE` environment
/// variable or `--sample-size` CLI argument.
///
/// ```
/// #[divan::bench_group(sample_size = 1000)]
/// mod math {
///     #[divan::bench]
///     fn add() -> i32 {
///         // ...
///         # 0
///     }
/// }
/// ```
///
/// ## `threads`
/// [`threads`]: #threads
///
/// See [`#[divan::bench(threads = ...)]`](macro@bench#threads).
///
/// ## `counters`
/// [`counters`]: #counters
///
/// The [`Counter`s](crate::counter::Counter) of each iteration of benchmarked
/// functions in a group can be set via the [`counters`] option. The following
/// example emits info for the number of bytes and number of ints processed when
/// benchmarking [slice sorting](slice::sort):
///
/// ```
/// use divan::{Bencher, counter::{BytesCount, ItemsCount}};
///
/// const INTS: &[i32] = &[
///     // ...
/// ];
///
/// #[divan::bench_group(counters = [
///     BytesCount::of_slice(INTS),
///     ItemsCount::new(INTS.len()),
/// ])]
/// mod sort {
///     use super::*;
///
///     #[divan::bench]
///     fn default(bencher: Bencher) {
///         bencher
///             .with_inputs(|| INTS.to_vec())
///             .bench_refs(|ints| ints.sort());
///     }
///
///     #[divan::bench]
///     fn unstable(bencher: Bencher) {
///         bencher
///             .with_inputs(|| INTS.to_vec())
///             .bench_refs(|ints| ints.sort_unstable());
///     }
/// }
/// # fn main() {}
/// ```
///
/// For convenience, singular `counter` allows a single
/// [`Counter`](crate::counter::Counter) to be set. The following example emits
/// info for the number of bytes processed when benchmarking
/// [`char`-counting](std::str::Chars::count) and
/// [`char`-collecting](std::str::Chars::collect):
///
/// ```
/// use divan::counter::BytesCount;
///
/// const STR: &str = "...";
///
/// #[divan::bench_group(counter = BytesCount::of_str(STR))]
/// mod chars {
///     use super::STR;
///
///     #[divan::bench]
///     fn count() -> usize {
///         divan::black_box(STR).chars().count()
///     }
///
///     #[divan::bench]
///     fn collect() -> String {
///         divan::black_box(STR).chars().collect()
///     }
/// }
/// # fn main() {}
/// ```
///
/// See:
/// - [`#[divan::bench(counters = ...)]`](macro@bench#counters)
/// - [`Bencher::counter`]
/// - [`Bencher::input_counter`]
///
/// ### `bytes_count`
/// [`bytes_count`]: #bytes_count
///
/// Convenience shorthand for
/// <code>[counter](#counters) = [BytesCount](counter::BytesCount)::from(n)</code>.
///
/// ### `chars_count`
/// [`chars_count`]: #chars_count
///
/// Convenience shorthand for
/// <code>[counter](#counters) = [CharsCount](counter::CharsCount)::from(n)</code>.
///
/// ### `cycles_count`
/// [`cycles_count`]: #cycles_count
///
/// Convenience shorthand for
/// <code>[counter](#counters) = [CyclesCount](counter::CyclesCount)::from(n)</code>.
///
/// ### `items_count`
/// [`items_count`]: #items_count
///
/// Convenience shorthand for
/// <code>[counter](#counters) = [ItemsCount](counter::ItemsCount)::from(n)</code>.
///
/// ## `min_time`
/// [`min_time`]: #min_time
///
/// The minimum time spent benchmarking each function can be set to a
/// predetermined [`Duration`] via the [`min_time`] option. This may be
/// overridden at runtime using either the `DIVAN_MIN_TIME` environment variable
/// or `--min-time` CLI argument.
///
/// Unless [`skip_ext_time`] is set, this includes time external to benchmarked
/// functions, such as time spent generating inputs and running [`Drop`].
///
/// ```
/// use std::time::Duration;
///
/// #[divan::bench_group(min_time = Duration::from_secs(3))]
/// mod math {
///     #[divan::bench]
///     fn add() -> i32 {
///         // ...
///         # 0
///     }
/// }
/// ```
///
/// For convenience, [`min_time`] can also be set with seconds as [`u64`] or
/// [`f64`]. Invalid values will cause a panic at runtime.
///
/// ```
/// #[divan::bench_group(min_time = 2)]
/// mod int_secs {
///     // ...
/// }
///
/// #[divan::bench_group(min_time = 1.5)]
/// mod float_secs {
///     // ...
/// }
/// ```
///
/// ## `max_time`
/// [`max_time`]: #max_time
///
/// The maximum time spent benchmarking each function can be set to a
/// predetermined [`Duration`] via the [`max_time`] option. This may be
/// overridden at runtime using either the `DIVAN_MAX_TIME` environment variable
/// or `--max-time` CLI argument.
///
/// Unless [`skip_ext_time`] is set, this includes time external to benchmarked
/// functions, such as time spent generating inputs and running [`Drop`].
///
/// If `min_time > max_time`, then [`max_time`] has priority and [`min_time`]
/// will not be reached.
///
/// ```
/// use std::time::Duration;
///
/// #[divan::bench_group(max_time = Duration::from_secs(5))]
/// mod math {
///     #[divan::bench]
///     fn add() -> i32 {
///         // ...
///         # 0
///     }
/// }
/// ```
///
/// For convenience, like [`min_time`], [`max_time`] can also be set with
/// seconds as [`u64`] or [`f64`]. Invalid values will cause a panic at runtime.
///
/// ```
/// #[divan::bench_group(max_time = 8)]
/// mod int_secs {
///     // ...
/// }
///
/// #[divan::bench_group(max_time = 9.5)]
/// mod float_secs {
///     // ...
/// }
/// ```
///
/// ## `skip_ext_time`
/// [`skip_ext_time`]: #skip_ext_time
///
/// By default, [`min_time`] and [`max_time`] include time external to
/// benchmarked functions, such as time spent generating inputs and running
/// [`Drop`]. Enabling the [`skip_ext_time`] option will instead make those
/// options only consider time spent within benchmarked functions. This may be
/// overridden at runtime using either the `DIVAN_SKIP_EXT_TIME` environment
/// variable or `--skip-ext-time` CLI argument.
///
/// In the following example, [`max_time`] only considers time spent running
/// `measured_function`:
///
/// ```
/// #[divan::bench_group(skip_ext_time)]
/// mod group {
///     # fn generate_input() {}
///     # fn measured_function(_: ()) {}
///     #[divan::bench(max_time = 5)]
///     fn bench(bencher: divan::Bencher) {
///         bencher
///             .with_inputs(|| generate_input())
///             .bench_values(|input| measured_function(input));
///     }
/// }
/// ```
///
/// This option can be set to an explicit [`bool`] value to override parent
/// values:
///
/// ```
/// #[divan::bench_group(skip_ext_time = false)]
/// mod group {
///     // ...
/// }
/// ```
///
/// ## `ignore`
/// [`ignore`]: #ignore
///
/// Like [`#[test]`](https://doc.rust-lang.org/reference/attributes/testing.html#the-test-attribute)
/// and [`#[divan::bench]`](macro@bench), `#[divan::bench_group]` functions can
/// use [`#[ignore]`](https://doc.rust-lang.org/reference/attributes/testing.html#the-ignore-attribute):
///
/// ```
/// #[divan::bench_group]
/// #[ignore]
/// mod math {
///     #[divan::bench]
///     fn todo() {
///         unimplemented!();
///     }
/// }
/// # divan::main();
/// ```
///
/// This option can also instead be set within the `#[divan::bench_group]`
/// attribute:
///
/// ```
/// #[divan::bench_group(ignore)]
/// mod math {
///     #[divan::bench]
///     fn todo() {
///         unimplemented!();
///     }
/// }
/// # divan::main();
/// ```
///
/// Like [`skip_ext_time`], this option can be set to an explicit [`bool`] value
/// to override parent values:
///
/// ```
/// #[divan::bench_group(ignore = false)]
/// mod group {
///     // ...
/// }
/// ```
///
/// This can be used to ignore benchmarks based on a runtime condition. The
/// following example benchmark group will be ignored if an [environment
/// variable](std::env::var) is not set to "true":
///
/// ```
/// #[divan::bench_group(
///     ignore = std::env::var("BENCH_EXPENSIVE").as_deref() != Ok("true")
/// )]
/// mod expensive_benches {
///     // ...
/// }
/// ```
///
/// [`Duration`]: std::time::Duration
pub use divan_macros::bench_group;
