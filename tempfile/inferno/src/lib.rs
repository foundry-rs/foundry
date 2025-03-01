//! Inferno is a set of tools that let you to produce [flame graphs] from performance profiles of
//! your application. It's a port of parts Brendan Gregg's original [flamegraph toolkit] that aims
//! to improve the performance of the original flamegraph tools and provide programmatic access to
//! them to facilitate integration with _other_ tools (like [not-perf]).
//!
//! Inferno, like the original flame graph toolkit, consists of two "stages": stack collapsing and
//! plotting. In the original Perl implementations, these were represented by the `stackcollapse-*`
//! binaries and `flamegraph.pl` respectively. In Inferno, collapsing is available through the
//! [`collapse`] module and the `inferno-collapse-*` binaries, and plotting can be found in the
//! [`flamegraph`] module and the `inferno-flamegraph` binary.
//!
//! # Command-line use
//!
//! ## Collapsing stacks
//!
//! Most sampling profilers (as opposed to [tracing profilers]) work by repeatedly recording the
//! state of the [call stack]. The stack can be sampled based on a fixed sampling interval, based
//! on [hardware or software events], or some combination of the two. In the end, you get a series
//! of [stack traces], each of which represents a snapshot of where the program was at different
//! points in time.
//!
//! Given enough of these snapshots, you can get a pretty good idea of where your program is
//! spending its time by looking at which functions appear in many of the traces. To ease this
//! analysis, we want to "collapse" the stack traces so if a particular trace occurs more than
//! once, we instead just keep it _once_ along with a count of how many times we've seen it. This
//! is what the various collapsing tools do! You'll sometimes see the resulting tuples of stack +
//! count called a "folded stack trace".
//!
//! Since profiling tools produce stack traces in a myriad of different formats, and the flame
//! graph plotter expects input in a particular folded stack trace format, each profiler needs a
//! separate collapse implementation. While the original Perl implementation supports _lots_ of
//! profilers, Inferno currently only supports four: the widely used [`perf`] tool (specifically
//! the output from `perf script`), [DTrace], [sample], and [VTune]. Support for xdebug is
//! [hopefully coming soon], and [`bpftrace`] should get [native support] before too long.
//!
//! Inferno supports profiles from applications written in any language, but we'll walk through an
//! example with a Rust program. To profile a Rust application, you would first set
//!
//! ```toml
//! [profile.release]
//! debug = true
//! ```
//!
//! in your `Cargo.toml` so that your profile will have useful function names and such included.
//! Then, compile with `--release`, and then run your favorite performance profiler:
//!
//! ### perf (Linux)
//!
//! ```console
//! # perf record --call-graph dwarf target/release/mybin
//! $ perf script | inferno-collapse-perf > stacks.folded
//! ```
//!
//! For more advanced uses, see Brendan Gregg's excellent [perf examples] page.
//!
//! Note: For larger binaries (like Firefox), the perf script can be significantly slowed down
//! by a non-optimal performance of the addr2line tool. Starting from perf version 6.12, you can
//! use an alternative addr2line tool (by using `perf script --addr2line=/path/to/addr2line`),
//! where the recommended one would be the Rust implementation from [Gimli project].
//!
//! ### DTrace (macOS)
//!
//! ```console
//! $ target/release/mybin &
//! $ pid=$!
//! # dtrace -x ustackframes=100 -n "profile-97 /pid == $pid/ { @[ustack()] = count(); } tick-60s { exit(0); }"  -o out.user_stacks
//! $ cat out.user_stacks | inferno-collapse-dtrace > stacks.folded
//! ```
//!
//! For more advanced uses, see also upstream FlameGraph's [DTrace examples].
//! You may also be interested in something like [NodeJS's ustack helper].
//!
//! ### sample (macOS)
//!
//! ```console
//! $ target/release/mybin &
//! $ pid=$!
//! $ sample $pid 30 -file sample.txt
//! $ inferno-collapse-sample sample.txt > stacks.folded
//! ```
//!
//! ### VTune (Windows and Linux)
//!
//! ```console
//! $ amplxe-cl -collect hotspots -r resultdir -- target/release/mybin
//! $ amplxe-cl -R top-down -call-stack-mode all -column=\"CPU Time:Self\",\"Module\" -report-out result.csv -filter \"Function Stack\" -format csv -csv-delimiter comma -r resultdir
//! $ inferno-collapse-vtune result.csv > stacks.folded
//! ```
//!
//! ## Producing a flame graph
//!
//! Once you have a folded stack file, you're ready to produce the flame graph SVG image. To do so,
//! simply provide the folded stack file to `inferno-flamegraph`, and it will print the resulting
//! SVG. Following on from the example above:
//!
//! ```console
//! $ cat stacks.folded | inferno-flamegraph > profile.svg
//! ```
//!
//! And then open `profile.svg` in your viewer of choice.
//!
//! ## Differential flame graphs
//!
//! You can debug CPU performance regressions with the help of differential flame graphs.
//! They let you easily visualize the differences between two profiles performed before and
//! after a code change. See Brendan Gregg's [differential flame graphs] blog post for a great
//! writeup. To create one you must first pass the two folded stack files to `inferno-diff-folded`,
//! then send the output to `inferno-flamegraph`. Example:
//!
//! ```console
//! $ inferno-diff-folded folded1 folded2 | inferno-flamegraph > diff2.svg
//! ```
//!
//! The flamegraph will be colored based on higher samples (red) and smaller samples (blue). The
//! frame widths will be based on the 2nd folded profile. This might be confusing if stack frames
//! disappear entirely; it will make the most sense to ALSO create a differential based on the 1st
//! profile widths, while switching the hues. To do this, reverse the order of the input files
//! and pass the `--negate` flag to `inferno-flamegraph` like this:
//!
//! ```console
//! $ inferno-diff-folded folded2 folded1 | inferno-flamegraph --negate > diff1.svg
//! ```
//!
//! # Feature flags
//! All features below are enabled by default
//! - `cli`: Also builds the `inferno` command-line tools
//! - `multithreaded`: Enables multithreaded stack-collapsing
//! - `nameattr`: Allows for adding customizing and adding attributes to the svg of [`flamegraph`]. See the `--nameattr` option for the flamegraph cli
//!
//! # Development
//!
//! This crate was initially developed through [a series of live coding sessions]. If you want to
//! contribute to the code, that may be a good way to learn why it's all designed the way it is!
//!
//!   [flame graphs]: http://www.brendangregg.com/flamegraphs.html
//!   [flamegraph toolkit]: https://github.com/brendangregg/FlameGraph
//!   [not-perf]: https://github.com/nokia/not-perf
//!   [tracing profilers]: https://danluu.com/perf-tracing/
//!   [call stack]: https://en.wikipedia.org/wiki/Call_stack
//!   [hardware or software events]: https://perf.wiki.kernel.org/index.php/Tutorial#Events
//!   [stack traces]: https://en.wikipedia.org/wiki/Stack_trace
//!   [`perf`]: https://perf.wiki.kernel.org/index.php/Main_Page
//!   [DTrace]: https://www.joyent.com/dtrace
//!   [hopefully coming soon]: https://twitter.com/DanielLockyer/status/1094605231155900416
//!   [native support]: https://github.com/jonhoo/inferno/issues/51#issuecomment-466732304
//!   [`bpftrace`]: https://github.com/iovisor/bpftrace
//!   [perf examples]: http://www.brendangregg.com/perf.html
//!   [DTrace examples]: http://www.brendangregg.com/FlameGraphs/cpuflamegraphs.html#DTrace
//!   [NodeJS's ustack helper]: http://dtrace.org/blogs/dap/2012/01/05/where-does-your-node-program-spend-its-time/
//!   [a series of live coding sessions]: https://www.youtube.com/watch?v=jTpK-bNZiA4&list=PLqbS7AVVErFimAvMW-kIJUwxpPvcPBCsz
//!   [differential flame graphs]: http://www.brendangregg.com/blog/2014-11-09/differential-flame-graphs.html
//!   [sample]: https://gist.github.com/loderunner/36724cc9ee8db66db305#profiling-with-sample
//!   [VTune]: https://software.intel.com/en-us/vtune-amplifier-help-command-line-interface
//!   [gimli project]: https://github.com/gimli-rs/addr2line

#![cfg_attr(doc, warn(rustdoc::all))]
#![cfg_attr(doc, allow(rustdoc::missing_doc_code_examples))]
#![deny(missing_docs)]
#![warn(unreachable_pub)]
#![allow(clippy::disallowed_names)]

/// Stack collapsing for various input formats.
///
/// See the [crate-level documentation] for details.
///
///   [crate-level documentation]: ../index.html
pub mod collapse;

/// Tool for creating an output required to generate differential flame graphs.
///
/// See the [crate-level documentation] for details.
///
///   [crate-level documentation]: ../index.html
pub mod differential;

/// Tools for producing flame graphs from folded stack traces.
///
/// See the [crate-level documentation] for details.
///
///   [crate-level documentation]: ../index.html
pub mod flamegraph;
