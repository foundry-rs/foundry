# Changelog [![crates.io][crate-badge]][crate]

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic
Versioning](http://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.17] - 2024-12-04

### Changed

- Set [MSRV] to 1.80 for `size_of` [`LazyLock`] and new prelude imports.

- Reduced thread pool memory usage by many kilobytes by using rendezvous
  channels instead of array-based channels.

## [0.1.16] - 2024-11-25

### Added

- Thread pool for reusing threads across multi-threaded benchmarks. The result
  is that when running Divan benchmarks under a sampling profiler, the
  profiler's output will be cleaner and easier to understand. ([#37])

- Track the maximum number of allocations during a benchmark.

### Changed

- Make private `Arg::get` trait method not take `self`, so that text editors
  don't recommend using it. ([#59])

- Cache `BenchOptions` using `LazyLock` instead of `OnceLock`, saving space and
  simplifying the implementation.

## [0.1.15] - 2024-10-31

### Added

- [`CyclesCount`] counter to display cycle throughput as Hertz.

- Track the maximum number of bytes allocated during a benchmark.

### Removed

- Remove `has_cpuid` polyfill due to it no longer being planned for Rust, since
  CPUID is assumed to be available on all old x86 Rust targets.

### Fixed

- List generic benchmark type parameter `A<4>` before `A<32>`. ([#64])

- Improve precision by using `f64` when calculating allocation count and sizes
  for the median samples.

- Multi-thread allocation counting in `sum_alloc_tallies` on macOS was loading a
  null pointer instead of the pointer initialized by `sync_threads`.

### Changes

- Sort all output benchmark names
  [naturally](https://en.wikipedia.org/wiki/Natural_sort_order) instead of
  [lexicographically](https://en.wikipedia.org/wiki/Lexicographic_order).

- Internally reuse [`&[&str]` slice][slice] for [`args`] names.

- Subtract overhead of [`AllocProfiler`] from timings. Now that Divan also
  tracks the maximum bytes allocated, the overhead was apparent in timings.

- Simplify `ThreadAllocInfo::clear`.

- Move measured loop overhead from `SharedContext` to global `OnceLock`.

- Macros no longer rely on `std` being re-exported by Divan. Instead they use
  `::std` or `::core` to greatly simplify code. Although this is technically a
  breaking change, it is extremely unlikely to do `extern crate std as x`.

## [0.1.14] - 2024-02-17

### Fixed

- Set correct field in [`Divan::max_time`]. ([#45](https://github.com/nvzqz/divan/pull/45))

### Changes

- Improve [`args`] documentation by relating it to using [`Bencher`].

- Define [`BytesCount::of_iter`] in terms of [`BytesCount::of_many`].

## [0.1.13] - 2024-02-09

### Fixed

- Missing update to `divan-macros` dependency.

## [0.1.12] - 2024-02-09

### Added

- Display [`args`] option values with [`Debug`] instead if [`ToString`] is not
  implemented.

  This makes it simple to use enums with derived [`Debug`]:

  ```rs
  #[derive(Debug)]
  enum Arg { A, B }

  #[divan::bench(args = [Arg::A, Arg::B])]
  fn bench_args(arg: &Arg) {
      ...
  }
  ```

- Documentation of when to use [`black_box`] in benchmarks.

## [0.1.11] - 2024-01-20

### Fixed

- Sorting negative [`args`] numbers.

## [0.1.10] - 2024-01-20

### Fixed

- Sort [`args`] numbers like [`consts`].

## [0.1.9] - 2024-01-20

### Added

- [`args`] option for providing runtime arguments to benchmarks:

  ```rs
  #[divan::bench(args = [1, 2, 3])]
  fn args_list(arg: usize) { ... }

  #[divan::bench(args = 1..=3)]
  fn args_range(arg: usize) { ... }

  const ARGS: &[usize] = [1, 2, 3];

  #[divan::bench(args = ARGS)]
  fn args_const(arg: usize) { ... }
  ```

  This option may be preferred over the similar [`consts`] option because:
  - It is compatible with more types, only requiring that the argument type
    implements [`Any`], [`Copy`], [`Send`], [`Sync`], and [`ToString`]. [`Copy`]
    is not needed if the argument is used through a reference.
  - It does not increase compile times, unlike [`consts`] which needs to
    generate new code for each constant used.

## [0.1.8] - 2023-12-19

### Changes

- Reduce [`AllocProfiler`] footprint from 6-10ns to 1-2ns:

  - Thread-local values are now exclusively owned by their threads and are no
    longer kept in a global list. This enables some optimizations:

    - Performing faster unsynchronized arithmetic.

    - Removing one level of pointer indirection by storing the thread-local
      value entirely inline in [`thread_local!`], rather than storing a pointer
      to a globally-shared instance.

    - Compiler emits SIMD arithmetic for x86_64 using `paddq`.

  - Improved thread-local lookup on x86_64 macOS by using a static lookup key
    instead of a dynamic key from [`pthread_key_create`]. Key 11 is used because
    it is reserved for Windows.

    The `dyn_thread_local` crate feature disables this optimization. This is
    recommended if your code or another dependency uses the same static key.

### Fixed

- Remove unused allocations if [`AllocProfiler`] is not active as the global
  allocator.

## [0.1.7] - 2023-12-13

### Changes

- Improve [`AllocProfiler`] implementation documentation.

- Limit [`AllocProfiler`] mean count outputs to 4 significant digits to not be
  very wide and for consistency with other outputs.

## [0.1.6] - 2023-12-13

### Added

- [`AllocProfiler`] allocator that tracks allocation counts and sizes during
  benchmarks.

## [0.1.5] - 2023-12-05

### Added

- [`black_box_drop`](https://docs.rs/divan/0.1.5/divan/fn.black_box_drop.html)
  convenience function for [`black_box`] + [`drop`]. This is useful when
  benchmarking a lazy [`Iterator`] to completion with `for_each`:

  ```rust
  #[divan::bench]
  fn parse_iter() {
      let input: &str = // ...

      Parser::new(input)
          .for_each(divan::black_box_drop);
  }
  ```

## [0.1.4] - 2023-12-02

### Added

- `From` implementations for counters on references to `u8`–`u64` and `usize`,
  such as `From<&u64>` and `From<&&u64>`. This allows for doing:

  ```rust
  bencher
      .with_inputs(|| { ... })
      .input_counter(ItemsCount::from)
      .bench_values(|n| { ... });
  ```

- [`Bencher::count_inputs_as<C>`](https://docs.rs/divan/0.1.4/divan/struct.Bencher.html#method.count_inputs_as)
  method to convert inputs to a `Counter`:

  ```rust
  bencher
      .with_inputs(|| -> usize {
          // ...
      })
      .count_inputs_as::<ItemsCount>()
      .bench_values(|n| -> Vec<usize> {
          (0..n).collect()
      });
  ```

## [0.1.3] - 2023-11-21

### Added

- Convenience shorthand options for `#[divan::bench]` and
  `#[divan::bench_group]` counters:
  - [`bytes_count`](https://docs.rs/divan/0.1.3/divan/attr.bench.html#bytes_count)
    for `counter = BytesCount::from(n)`
  - [`chars_count`](https://docs.rs/divan/0.1.3/divan/attr.bench.html#chars_count)
    for `counter = CharsCount::from(n)`
  - [`items_count`](https://docs.rs/divan/0.1.3/divan/attr.bench.html#items_count)
    for `counter = ItemsCount::from(n)`

- Support for NetBSD, DragonFly BSD, and Haiku OS by using pre-`main`.

- Set global thread counts using:
  - [`Divan::threads`](https://docs.rs/divan/0.1.3/divan/struct.Divan.html#method.threads)
  - `--threads A B C...` CLI arg
  - `DIVAN_THREADS=A,B,C` env var

  The following example will benchmark across 2, 4, and [available parallelism]
  thread counts:

  ```sh
  DIVAN_THREADS=0,2,4 cargo bench -q -p examples --bench atomic
  ```

- Set global
  [`Counter`s](https://docs.rs/divan/0.1.3/divan/counter/trait.Counter.html) at
  runtime using:
  - [`Divan::counter`](https://docs.rs/divan/0.1.3/divan/struct.Divan.html#method.counter)
  - [`Divan::items_count`](https://docs.rs/divan/0.1.3/divan/struct.Divan.html#method.items_count)
  - [`Divan::bytes_count`](https://docs.rs/divan/0.1.3/divan/struct.Divan.html#method.bytes_count)
  - [`Divan::chars_count`](https://docs.rs/divan/0.1.3/divan/struct.Divan.html#method.chars_count)
  - `--items-count N` CLI arg
  - `--bytes-count N` CLI arg
  - `--chars-count N` CLI arg
  - `DIVAN_ITEMS_COUNT=N` env var
  - `DIVAN_BYTES_COUNT=N` env var
  - `DIVAN_CHARS_COUNT=N` env var

- `From<C>` for
  [`ItemsCount`](https://docs.rs/divan/0.1.3/divan/counter/struct.ItemsCount.html),
  [`BytesCount`](https://docs.rs/divan/0.1.3/divan/counter/struct.BytesCount.html),
  and
  [`CharsCount`](https://docs.rs/divan/0.1.3/divan/counter/struct.CharsCount.html)
  where `C` is `u8`–`u64` or `usize` (via `CountUInt` internally). This provides
  an alternative to the `new` constructor.

- [`BytesCount::of_many`](https://docs.rs/divan/0.1.3/divan/counter/struct.BytesCount.html#method.of_many)
  method similar to [`BytesCount::of`](https://docs.rs/divan/0.1/divan/counter/struct.BytesCount.html#method.of),
  but with a parameter by which to multiply the size of the type.

- [`BytesCount::u64`](https://docs.rs/divan/0.1.3/divan/counter/struct.BytesCount.html#method.u64),
  [`BytesCount::f64`](https://docs.rs/divan/0.1.3/divan/counter/struct.BytesCount.html#method.f64),
  and similar methods based on [`BytesCount::of_many`](https://docs.rs/divan/0.1.3/divan/counter/struct.BytesCount.html#method.of_many).

### Removed

- [`black_box`] inside benchmark loop when deferring [`Drop`] of outputs. This
  is now done after the loop.

- [`linkme`](https://docs.rs/linkme) dependency in favor of pre-`main` to
  register benchmarks and benchmark groups. This is generally be more portable
  and reliable.

### Changed

- Now calling [`black_box`] at the end of the benchmark loop when deferring use
  of inputs or [`Drop`] of outputs.

## [0.1.2] - 2023-10-28

### Fixed

- Multi-threaded benchmarks being spread across CPUs, instead of pinning the
  main thread to CPU 0 and having all threads inherit the main thread's
  affinity.

## [0.1.1] - 2023-10-25

### Fixed

- Fix using LLD as linker for Linux by using the same pre-`main` approach as
  Windows.

## 0.1.0 - 2023-10-04

Initial release. See [blog post](https://nikolaivazquez.com/blog/divan/).

[crate]:       https://crates.io/crates/divan
[crate-badge]: https://img.shields.io/crates/v/divan.svg

[Unreleased]: https://github.com/nvzqz/divan/compare/v0.1.17...HEAD
[0.1.17]: https://github.com/nvzqz/divan/compare/v0.1.16...v0.1.17
[0.1.16]: https://github.com/nvzqz/divan/compare/v0.1.15...v0.1.16
[0.1.15]: https://github.com/nvzqz/divan/compare/v0.1.14...v0.1.15
[0.1.14]: https://github.com/nvzqz/divan/compare/v0.1.13...v0.1.14
[0.1.13]: https://github.com/nvzqz/divan/compare/v0.1.12...v0.1.13
[0.1.12]: https://github.com/nvzqz/divan/compare/v0.1.11...v0.1.12
[0.1.11]: https://github.com/nvzqz/divan/compare/v0.1.10...v0.1.11
[0.1.10]: https://github.com/nvzqz/divan/compare/v0.1.9...v0.1.10
[0.1.9]: https://github.com/nvzqz/divan/compare/v0.1.8...v0.1.9
[0.1.8]: https://github.com/nvzqz/divan/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/nvzqz/divan/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/nvzqz/divan/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/nvzqz/divan/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/nvzqz/divan/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/nvzqz/divan/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/nvzqz/divan/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/nvzqz/divan/compare/v0.1.0...v0.1.1

[#37]: https://github.com/nvzqz/divan/issues/37
[#59]: https://github.com/nvzqz/divan/issues/59
[#64]: https://github.com/nvzqz/divan/issues/64

[`AllocProfiler`]: https://docs.rs/divan/0.1/divan/struct.AllocProfiler.html
[`args`]: https://docs.rs/divan/latest/divan/attr.bench.html#args
[`Bencher`]: https://docs.rs/divan/0.1/divan/struct.Bencher.html
[`black_box`]: https://docs.rs/divan/latest/divan/fn.black_box.html
[`BytesCount::of_iter`]: https://docs.rs/divan/0.1/divan/counter/struct.BytesCount.html#method.of_iter
[`BytesCount::of_many`]: https://docs.rs/divan/0.1/divan/counter/struct.BytesCount.html#method.of_many
[`consts`]: https://docs.rs/divan/latest/divan/attr.bench.html#consts
[`CyclesCount`]: https://docs.rs/divan/0.1/divan/counter/struct.CyclesCount.html
[`Divan::max_time`]: https://docs.rs/divan/0.1/divan/struct.Divan.html#method.max_time

[`Any`]: https://doc.rust-lang.org/std/any/trait.Any.html
[`Copy`]: https://doc.rust-lang.org/std/marker/trait.Copy.html
[`Debug`]: https://doc.rust-lang.org/std/fmt/trait.Debug.html
[`drop`]: https://doc.rust-lang.org/std/mem/fn.drop.html
[`Drop`]: https://doc.rust-lang.org/std/ops/trait.Drop.html
[`Iterator`]: https://doc.rust-lang.org/std/iter/trait.Iterator.html
[`LazyLock`]: https://doc.rust-lang.org/std/sync/struct.LazyLock.html
[`Send`]: https://doc.rust-lang.org/std/marker/trait.Send.html
[`size_of`]: https://doc.rust-lang.org/std/mem/fn.size_of.html
[`Sync`]: https://doc.rust-lang.org/std/marker/trait.Sync.html
[`thread_local!`]: https://doc.rust-lang.org/std/macro.thread_local.html
[`ToString`]: https://doc.rust-lang.org/std/string/trait.ToString.html
[available parallelism]: https://doc.rust-lang.org/std/thread/fn.available_parallelism.html
[slice]: https://doc.rust-lang.org/std/primitive.slice.html

[MSRV]: https://doc.rust-lang.org/cargo/reference/rust-version.html

[`pthread_key_create`]: https://pubs.opengroup.org/onlinepubs/9699919799/functions/pthread_key_create.html
