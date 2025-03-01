//! indicatif is a library for Rust that helps you build command line
//! interfaces that report progress to users.  It comes with various
//! tools and utilities for formatting anything that indicates progress.
//!
//! Platform support:
//!
//! * Linux
//! * macOS
//! * Windows (colors require Windows 10)
//!
//! Best paired with other libraries in the family:
//!
//! * [console](https://docs.rs/console)
//! * [dialoguer](https://docs.rs/dialoguer)
//!
//! # Crate Contents
//!
//! * **Progress bars**
//!   * [`ProgressBar`](struct.ProgressBar.html) for bars and spinners
//!   * [`MultiProgress`](struct.MultiProgress.html) for multiple bars
//! * **Data Formatting**
//!   * [`HumanBytes`](struct.HumanBytes.html) for formatting bytes
//!   * [`DecimalBytes`](struct.DecimalBytes.html) for formatting bytes using SI prefixes
//!   * [`BinaryBytes`](struct.BinaryBytes.html) for formatting bytes using ISO/IEC prefixes
//!   * [`HumanDuration`](struct.HumanDuration.html) for formatting durations
//!   * [`HumanCount`](struct.HumanCount.html) for formatting large counts
//!   * [`HumanFloatCount`](struct.HumanFloatCount.html) for formatting large float counts
//!
//! # Progress Bars and Spinners
//!
//! indicatif comes with a [`ProgressBar`] type that supports both bounded
//! progress bar uses as well as unbounded "spinner" type progress reports.
//! Progress bars are [`Sync`] and [`Send`] objects which means that they are
//! internally locked and can be passed from thread to thread.
//!
//! Additionally a [`MultiProgress`] utility is provided that can manage
//! rendering multiple progress bars at once (eg: from multiple threads).
//!
//! To whet your appetite, this is what this can look like:
//!
//! <img src="https://github.com/console-rs/indicatif/raw/main/screenshots/yarn.gif?raw=true" width="60%">
//!
//! Progress bars are manually advanced and by default draw to stderr.
//! When you are done, the progress bar can be finished either visibly
//! (eg: the progress bar stays on the screen) or cleared (the progress
//! bar will be removed).
//!
//! ```rust
//! use indicatif::ProgressBar;
//!
//! let bar = ProgressBar::new(1000);
//! for _ in 0..1000 {
//!     bar.inc(1);
//!     // ...
//! }
//! bar.finish();
//! ```
//!
//! Spinners can be manually advanced with [`tick`](ProgressBar::tick), or you can set them up
//! to spin automatically with [`enable_steady_tick`](ProgressBar::enable_steady_tick):
//!
//! ```rust
//! use std::time::Duration;
//! use indicatif::ProgressBar;
//!
//! let bar = ProgressBar::new_spinner();
//! bar.enable_steady_tick(Duration::from_millis(100));
//! // ... do some work
//! bar.finish();
//! ```
//!
//! General progress bar behaviors:
//!
//! * if a non terminal is detected the progress bar will be completely
//!   hidden.  This makes piping programs to logfiles make sense out of
//!   the box.
//! * a progress bar only starts drawing when [`set_message`](ProgressBar::set_message),
//!   [`inc`](ProgressBar::inc), [`set_position`](ProgressBar::set_position)
//!   or [`tick`](ProgressBar::tick) are called. In some situations you
//!   might have to call [`tick`](ProgressBar::tick) once to draw it.
//! * progress bars should be explicitly finished to reset the rendering
//!   for others.  Either by also clearing them or by replacing them with
//!   a new message / retaining the current message.
//! * the default template renders neither message nor prefix.
//!
//! # Iterators
//!
//! Similar to [tqdm](https://github.com/tqdm/tqdm), progress bars can be
//! associated with an iterator. For example:
//!
//! ```rust
//! use indicatif::ProgressIterator;
//!
//! for _ in (0..1000).progress() {
//!     // ...
//! }
//! ```
//!
//! See the [`ProgressIterator`](trait.ProgressIterator.html) trait for more
//! methods to configure the number of elements in the iterator or change
//! the progress bar style. Indicatif also has optional support for parallel
//! iterators with [Rayon](https://github.com/rayon-rs/rayon). In your
//! `Cargo.toml`, use the "rayon" feature:
//!
//! ```toml
//! [dependencies]
//! indicatif = {version = "*", features = ["rayon"]}
//! ```
//!
//! And then use it like this:
//!
//! ```rust,ignore
//! # extern crate rayon;
//! use indicatif::ParallelProgressIterator;
//! use rayon::iter::{ParallelIterator, IntoParallelRefIterator};
//!
//! let v: Vec<_> = (0..100000).collect();
//! let v2: Vec<_> = v.par_iter().progress_count(v.len() as u64).map(|i| i + 1).collect();
//! assert_eq!(v2[0], 1);
//! ```
//!
//! Or if you'd like to customize the progress bar:
//!
//! ```rust,ignore
//! # extern crate rayon;
//! use indicatif::{ProgressBar, ParallelProgressIterator, ProgressStyle};
//! use rayon::iter::{ParallelIterator, IntoParallelRefIterator};
//!
//! // Alternatively, use `ProgressBar::new().with_style()`
//! let style = ProgressStyle::default_bar();
//! let v: Vec<_> = (0..100000).collect();
//! let v2: Vec<_> = v.par_iter().progress_with_style(style).map(|i| i + 1).collect();
//! assert_eq!(v2[0], 1);
//! ```
//!
//! # Templates
//!
//! Progress bars can be styled with simple format strings similar to the
//! ones in Rust itself.  The format for a placeholder is `{key:options}`
//! where the `options` part is optional.  If provided the format is this:
//!
//! ```text
//! <^>             for an optional alignment specification (left, center and right respectively)
//! WIDTH           an optional width as positive integer
//! !               an optional exclamation mark to enable truncation
//! .STYLE          an optional dot separated style string
//! /STYLE          an optional dot separated alternative style string
//! ```
//!
//! For the style component see [`Style::from_dotted_str`](https://docs.rs/console/0.7.5/console/struct.Style.html#method.from_dotted_str)
//! for more information. Indicatif uses the `console` base crate for all
//! colorization and formatting options.
//!
//! Some examples for templates:
//!
//! ```text
//! [{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}
//! ```
//!
//! This sets a progress bar that is 40 characters wide and has cyan
//! as primary style color and blue as alternative style color.
//! Alternative styles are currently only used for progress bars.
//!
//! Example configuration:
//!
//! ```rust
//! # use indicatif::{ProgressBar, ProgressStyle};
//! # let bar = ProgressBar::new(0);
//! bar.set_style(ProgressStyle::with_template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
//!     .unwrap()
//!     .progress_chars("##-"));
//! ```
//!
//! The following keys exist:
//!
//! * `bar`: renders a progress bar. By default 20 characters wide.  The
//!   style string is used to color the elapsed part, the alternative
//!   style is used for the bar that is yet to render.
//! * `wide_bar`: like `bar` but always fills the remaining space. It should not be used with `wide_msg`.
//! * `spinner`: renders the spinner (current tick string). Note that spinners do not automatically tick by default. You either
//!   need to call `enable_steady_tick` or manually call `tick`.
//! * `prefix`: renders the prefix set on the progress bar.
//! * `msg`: renders the currently set message on the progress bar.
//! * `wide_msg`: like `msg` but always fills the remaining space and truncates. It should not be used
//!   with `wide_bar`.
//! * `pos`: renders the current position of the bar as integer
//! * `human_pos`: renders the current position of the bar as an integer, with commas as the
//!   thousands separator.
//! * `len`: renders the amount of work to be done as an integer
//! * `human_len`: renders the total length of the bar as an integer, with commas as the thousands
//!   separator.
//! * `percent`: renders the current position of the bar as a percentage of the total length (as an integer).
//! * `percent_precise`: renders the current position of the bar as a percentage of the total length (with 3 fraction digits).
//! * `bytes`: renders the current position of the bar as bytes (alias of `binary_bytes`).
//! * `total_bytes`: renders the total length of the bar as bytes (alias of `binary_total_bytes`).
//! * `decimal_bytes`: renders the current position of the bar as bytes using
//!   power-of-10 units, i.e. `MB`, `kB`, etc.
//! * `decimal_total_bytes`: renders the total length of the bar as bytes using
//!   power-of-10 units, i.e. `MB`, `kB`, etc.
//! * `binary_bytes`: renders the current position of the bar as bytes using
//!   power-of-two units, i.e. `MiB`, `KiB`, etc.
//! * `binary_total_bytes`: renders the total length of the bar as bytes using
//!   power-of-two units, i.e. `MiB`, `KiB`, etc.
//! * `elapsed_precise`: renders the elapsed time as `HH:MM:SS`.
//! * `elapsed`: renders the elapsed time as `42s`, `1m` etc.
//! * `per_sec`: renders the speed in steps per second.
//! * `bytes_per_sec`: renders the speed in bytes per second (alias of `binary_bytes_per_sec`).
//! * `decimal_bytes_per_sec`: renders the speed in bytes per second using
//!   power-of-10 units, i.e. `MB`, `kB`, etc.
//! * `binary_bytes_per_sec`: renders the speed in bytes per second using
//!   power-of-two units, i.e. `MiB`, `KiB`, etc.
//! * `eta_precise`: the remaining time (like `elapsed_precise`).
//! * `eta`: the remaining time (like `elapsed`).
//! * `duration_precise`: the extrapolated total duration (like `elapsed_precise`).
//! * `duration`: the extrapolated total duration time (like `elapsed`).
//!
//! If the list above does not contain the value you need, consider creating a custom
//! [`ProgressTracker`][crate::style::ProgressTracker] implementation.
//!
//! The design of the progress bar can be altered with the integrated
//! template functionality.  The template can be set by changing a
//! [`ProgressStyle`] and attaching it to the progress bar.
//!
//! # Human Readable Formatting
//!
//! There are some formatting wrappers for showing elapsed time and
//! file sizes for human users:
//!
//! ```rust
//! # use std::time::Duration;
//! use indicatif::{HumanBytes, HumanCount, HumanDuration, HumanFloatCount};
//!
//! assert_eq!("3.00 MiB", HumanBytes(3*1024*1024).to_string());
//! assert_eq!("8 seconds", HumanDuration(Duration::from_secs(8)).to_string());
//! assert_eq!("33,857,009", HumanCount(33857009).to_string());
//! assert_eq!("33,857,009.1235", HumanFloatCount(33857009.123456).to_string());
//! ```
//!
//! # Feature Flags
//!
//! * `rayon`: adds rayon support
//! * `improved_unicode`: adds improved unicode support (graphemes, better width calculation)

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(unreachable_pub)]

mod draw_target;
mod format;
#[cfg(feature = "in_memory")]
mod in_memory;
mod iter;
mod multi;
mod progress_bar;
#[cfg(feature = "rayon")]
mod rayon;
mod state;
pub mod style;
mod term_like;

pub use crate::draw_target::ProgressDrawTarget;
pub use crate::format::{
    BinaryBytes, DecimalBytes, FormattedDuration, HumanBytes, HumanCount, HumanDuration,
    HumanFloatCount,
};
#[cfg(feature = "in_memory")]
pub use crate::in_memory::InMemoryTerm;
pub use crate::iter::{ProgressBarIter, ProgressIterator};
pub use crate::multi::{MultiProgress, MultiProgressAlignment};
pub use crate::progress_bar::{ProgressBar, WeakProgressBar};
#[cfg(feature = "rayon")]
pub use crate::rayon::ParallelProgressIterator;
pub use crate::state::{ProgressFinish, ProgressState};
pub use crate::style::ProgressStyle;
pub use crate::term_like::TermLike;

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    trait MustBeThreadSafe: Send + Sync {}

    // Ensure that the following types are `Send + Sync`
    impl MustBeThreadSafe for MultiProgress {}
    impl MustBeThreadSafe for MultiProgressAlignment {}
    impl MustBeThreadSafe for ProgressBar {}
    impl MustBeThreadSafe for ProgressBarIter<()> {}
    impl MustBeThreadSafe for ProgressFinish {}
    impl MustBeThreadSafe for ProgressState {}
    impl MustBeThreadSafe for ProgressStyle {}
    impl MustBeThreadSafe for WeakProgressBar {}
}
