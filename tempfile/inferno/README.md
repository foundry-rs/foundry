[![Crates.io](https://img.shields.io/crates/v/inferno.svg)](https://crates.io/crates/inferno)
[![Documentation](https://docs.rs/inferno/badge.svg)](https://docs.rs/inferno/)
[![Codecov](https://codecov.io/github/jonhoo/inferno/coverage.svg?branch=master)](https://codecov.io/gh/jonhoo/inferno)
[![Dependency status](https://deps.rs/repo/github/jonhoo/inferno/status.svg)](https://deps.rs/repo/github/jonhoo/inferno)

Inferno is a port of parts of the [flamegraph
toolkit](http://www.brendangregg.com/flamegraphs.html) to Rust, with the
aim of improving the performance of the original flamegraph tools. The
primary focus is on speeding up the `stackcollapse-*` tools that process
output from various profiling tools into the "folded" format expected by
the `flamegraph` plotting tool. So far, the focus has been on parsing
profiling results from
[`perf`](https://perf.wiki.kernel.org/index.php/Main_Page) and
[DTrace](https://www.joyent.com/dtrace). At the time of writing,
`inferno-collapse-perf` is ~20x faster than `stackcollapse-perf.pl` and
`inferno-collapse-dtrace` is ~20x faster than `stackcollapse.pl` (see
`compare.sh`).

It is developed in part through live coding sessions, which you can find
[on YouTube](https://www.youtube.com/watch?v=jTpK-bNZiA4&list=PLqbS7AVVErFimAvMW-kIJUwxpPvcPBCsz).

## Using Inferno

### As a library

Inferno provides a [library interface](https://docs.rs/inferno/) through
the `inferno` crate. This will let you collapse stacks and produce flame
graphs without going through the command line, and is intended for
integration with external Rust tools like [`cargo-flamegraph`].

  [`cargo-flamegraph`]: https://github.com/ferrous-systems/cargo-flamegraph

### As a binary

First of all, you may want to look into [cargo
flamegraph](https://github.com/ferrous-systems/cargo-flamegraph/), which
deals with much of the infrastructure for you!

If you want to use Inferno directly, then build your application in
release mode and with debug symbols, and then [run a profiler] to gather
profiling data. Once you have the data, pass it through the appropriate
Inferno "collapser". Depending on your platform, this will look
something like

  [run a profiler]: http://www.brendangregg.com/FlameGraphs/cpuflamegraphs.html#Instructions

```console
$ # Linux
# perf record --call-graph dwarf target/release/mybin
$ perf script | inferno-collapse-perf > stacks.folded
```

or

```console
$ # macOS
$ target/release/mybin &
$ pid=$!
# dtrace -x ustackframes=100 -n "profile-97 /pid == $pid/ { @[ustack()] = count(); } tick-60s { exit(0); }"  -o out.user_stacks
$ cat out.user_stacks | inferno-collapse-dtrace > stacks.folded
```

You can also use `inferno-collapse-guess` which should work on both
perf and DTrace samples. In the end, you'll end up with a "folded stack"
file. You can pass that file to `inferno-flamegraph` to generate a flame
graph SVG:

```console
$ cat stacks.folded | inferno-flamegraph > flamegraph.svg
```

You'll end up with an image like this:

[![colorized flamegraph output](tests/data/flamegraph/example-perf-stacks/example-perf-stacks.svg)](tests/data/flamegraph/example-perf-stacks/example-perf-stacks.svg)

### Obtaining profiling data

To profile your application, you'll need to have a "profiler" installed.
This will likely be [`perf`]() or [`bpftrace`] on Linux, and [DTrace] on
macOS. There are some great instructions on how to get started with
these tools on Brendan Gregg's [CPU Flame Graphs page].

  [profiler]: https://en.wikipedia.org/wiki/Profiling_(computer_programming
  [`perf`]: https://perf.wiki.kernel.org/index.php/Main_Page
  [`bpftrace`]: https://github.com/iovisor/bpftrace/
  [DTrace]: https://www.joyent.com/dtrace
  [CPU Flame Graphs page]: http://www.brendangregg.com/FlameGraphs/cpuflamegraphs.html#Instructions

On Linux, you may need to tweak a kernel config such as
```console
$ echo 0 | sudo tee /proc/sys/kernel/perf_event_paranoid
```
to get profiling [to work](https://unix.stackexchange.com/a/14256).

## Performance

### Comparison to the Perl implementation

To run Inferno's performance comparison, run `./compare.sh`.
It requires [hyperfine](https://github.com/sharkdp/hyperfine), and you
must make sure you also check out Inferno's
[submodules](https://github.blog/2016-02-01-working-with-submodules/).
In general, Inferno's perf and dtrace collapsers are ~20x faster than
`stackcollapse-*`, and the sample collapser is ~10x faster.

### Benchmarks

Inferno includes [criterion](https://github.com/bheisler/criterion.rs)
benchmarks in [`benches/`](benches/). Criterion saves its results in
`target/criterion/`, and uses that to recognize changes in performance,
which should make it easy to detect performance regressions while
developing bugfixes and improvements.

You can run the benchmarks with `cargo bench`. Some results (YMMV):

My desktop computer (AMD Ryzen 5 2600X) gets (`/N` means `N` cores):

```
collapse/dtrace/1       time:   [8.2767 ms 8.2817 ms 8.2878 ms]
                        thrpt:  [159.08 MiB/s 159.20 MiB/s 159.29 MiB/s]
collapse/dtrace/12      time:   [3.8631 ms 3.8819 ms 3.9019 ms]
                        thrpt:  [337.89 MiB/s 339.63 MiB/s 341.28 MiB/s]

collapse/perf/1         time:   [16.386 ms 16.401 ms 16.416 ms]
                        thrpt:  [182.37 MiB/s 182.53 MiB/s 182.70 MiB/s]
collapse/perf/12        time:   [4.8056 ms 4.8254 ms 4.8460 ms]
                        thrpt:  [617.78 MiB/s 620.41 MiB/s 622.97 MiB/s]

collapse/sample         time:   [8.9132 ms 8.9196 ms 8.9264 ms]
                        thrpt:  [155.49 MiB/s 155.61 MiB/s 155.72 MiB/s]

flamegraph              time:   [16.071 ms 16.118 ms 16.215 ms]
                        thrpt:  [38.022 MiB/s 38.250 MiB/s 38.363 MiB/s]
```

My laptop (Intel Core i7-8650U) gets:

```
collapse/dtrace/1       time:   [8.3612 ms 8.3839 ms 8.4114 ms]
                        thrpt:  [156.74 MiB/s 157.25 MiB/s 157.68 MiB/s]
collapse/dtrace/8       time:   [3.4623 ms 3.4826 ms 3.5014 ms]
                        thrpt:  [376.54 MiB/s 378.58 MiB/s 380.79 MiB/s]

collapse/perf/1         time:   [15.723 ms 15.756 ms 15.798 ms]
                        thrpt:  [189.51 MiB/s 190.01 MiB/s 190.41 MiB/s]
collapse/perf/8         time:   [6.1391 ms 6.1554 ms 6.1715 ms]
                        thrpt:  [485.09 MiB/s 486.36 MiB/s 487.65 MiB/s]

collapse/sample         time:   [9.3194 ms 9.3429 ms 9.3719 ms]
                        thrpt:  [148.10 MiB/s 148.56 MiB/s 148.94 MiB/s]

flamegraph              time:   [16.490 ms 16.503 ms 16.518 ms]
                        thrpt:  [37.324 MiB/s 37.358 MiB/s 37.388 MiB/s]
```

## License

Inferno is a port of @brendangregg's awesome original
[FlameGraph](https://github.com/brendangregg/FlameGraph) project,
written in Perl, and owes its existence and pretty much of all of its
functionality entirely to that project. [Like
FlameGraph](https://github.com/brendangregg/FlameGraph/commit/76719a446d6091c88434489cc99d6355c3c3ef41),
Inferno is licensed under the [CDDL
1.0](https://opensource.org/licenses/CDDL-1.0) to avoid any licensing
issues. Specifically, the CDDL 1.0 grants

> a world-wide, royalty-free, non-exclusive license under intellectual
> property rights (other than patent or trademark) Licensable by Initial
> Developer, to use, reproduce, modify, display, perform, sublicense and
> distribute the Original Software (or portions thereof), with or
> without Modifications, and/or as part of a Larger Work; and under
> Patent Claims infringed by the making, using or selling of Original
> Software, to make, have made, use, practice, sell, and offer for sale,
> and/or otherwise dispose of the Original Software (or portions
> thereof).

as long as the source is made available along with the license (3.1),
both of which are true since you're reading this file!
