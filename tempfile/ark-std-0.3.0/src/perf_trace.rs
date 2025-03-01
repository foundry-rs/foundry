#![allow(unused_imports)]
//! This module contains macros for logging to stdout a trace of wall-clock time required
//! to execute annotated code. One can use this code as follows:
//! ```
//! use ark_std::{start_timer, end_timer};
//! let start = start_timer!(|| "Addition of two integers");
//! let c = 5 + 7;
//! end_timer!(start);
//! ```
//! The foregoing code should log the following to stdout.
//! ```text
//! Start: Addition of two integers
//! End: Addition of two integers... 1ns
//! ```
//!
//! These macros can be arbitrarily nested, and the nested nature is made apparent
//! in the output. For example, the following snippet:
//! ```
//! use ark_std::{start_timer, end_timer};
//! let start = start_timer!(|| "Addition of two integers");
//! let start2 = start_timer!(|| "Inner");
//! let c = 5 + 7;
//! end_timer!(start2);
//! end_timer!(start);
//! ```
//! should print out the following:
//! ```text
//! Start: Addition of two integers
//!     Start: Inner
//!     End: Inner               ... 1ns
//! End: Addition of two integers... 1ns
//! ```
//!
//! Additionally, one can use the `add_to_trace` macro to log additional context
//! in the output.
pub use self::inner::*;

#[macro_use]
#[cfg(feature = "print-trace")]
pub mod inner {
    pub use colored::Colorize;

    // print-trace requires std, so these imports are well-defined
    pub use std::{
        format, println,
        string::{String, ToString},
        sync::atomic::{AtomicUsize, Ordering},
        time::Instant,
    };

    pub static NUM_INDENT: AtomicUsize = AtomicUsize::new(0);
    pub const PAD_CHAR: &str = "·";

    pub struct TimerInfo {
        pub msg: String,
        pub time: Instant,
    }

    #[macro_export]
    macro_rules! start_timer {
        ($msg:expr) => {{
            use $crate::perf_trace::inner::{
                compute_indent, AtomicUsize, Colorize, Instant, Ordering, ToString, NUM_INDENT,
                PAD_CHAR,
            };

            let msg = $msg();
            let start_info = "Start:".yellow().bold();
            let indent_amount = 2 * NUM_INDENT.fetch_add(0, Ordering::Relaxed);
            let indent = compute_indent(indent_amount);

            $crate::perf_trace::println!("{}{:8} {}", indent, start_info, msg);
            NUM_INDENT.fetch_add(1, Ordering::Relaxed);
            $crate::perf_trace::TimerInfo {
                msg: msg.to_string(),
                time: Instant::now(),
            }
        }};
    }

    #[macro_export]
    macro_rules! end_timer {
        ($time:expr) => {{
            end_timer!($time, || "");
        }};
        ($time:expr, $msg:expr) => {{
            use $crate::perf_trace::inner::{
                compute_indent, format, AtomicUsize, Colorize, Instant, Ordering, ToString,
                NUM_INDENT, PAD_CHAR,
            };

            let time = $time.time;
            let final_time = time.elapsed();
            let final_time = {
                let secs = final_time.as_secs();
                let millis = final_time.subsec_millis();
                let micros = final_time.subsec_micros() % 1000;
                let nanos = final_time.subsec_nanos() % 1000;
                if secs != 0 {
                    format!("{}.{:03}s", secs, millis).bold()
                } else if millis > 0 {
                    format!("{}.{:03}ms", millis, micros).bold()
                } else if micros > 0 {
                    format!("{}.{:03}µs", micros, nanos).bold()
                } else {
                    format!("{}ns", final_time.subsec_nanos()).bold()
                }
            };

            let end_info = "End:".green().bold();
            let message = format!("{} {}", $time.msg, $msg());

            NUM_INDENT.fetch_sub(1, Ordering::Relaxed);
            let indent_amount = 2 * NUM_INDENT.fetch_add(0, Ordering::Relaxed);
            let indent = compute_indent(indent_amount);

            // Todo: Recursively ensure that *entire* string is of appropriate
            // width (not just message).
            $crate::perf_trace::println!(
                "{}{:8} {:.<pad$}{}",
                indent,
                end_info,
                message,
                final_time,
                pad = 75 - indent_amount
            );
        }};
    }

    #[macro_export]
    macro_rules! add_to_trace {
        ($title:expr, $msg:expr) => {{
            use $crate::perf_trace::{
                compute_indent, compute_indent_whitespace, format, AtomicUsize, Colorize, Instant,
                Ordering, ToString, NUM_INDENT, PAD_CHAR,
            };

            let start_msg = "StartMsg".yellow().bold();
            let end_msg = "EndMsg".green().bold();
            let title = $title();
            let start_msg = format!("{}: {}", start_msg, title);
            let end_msg = format!("{}: {}", end_msg, title);

            let start_indent_amount = 2 * NUM_INDENT.fetch_add(0, Ordering::Relaxed);
            let start_indent = compute_indent(start_indent_amount);

            let msg_indent_amount = 2 * NUM_INDENT.fetch_add(0, Ordering::Relaxed) + 2;
            let msg_indent = compute_indent_whitespace(msg_indent_amount);
            let mut final_message = "\n".to_string();
            for line in $msg().lines() {
                final_message += &format!("{}{}\n", msg_indent, line,);
            }

            // Todo: Recursively ensure that *entire* string is of appropriate
            // width (not just message).
            $crate::perf_trace::println!("{}{}", start_indent, start_msg);
            $crate::perf_trace::println!("{}{}", msg_indent, final_message,);
            $crate::perf_trace::println!("{}{}", start_indent, end_msg);
        }};
    }

    pub fn compute_indent_whitespace(indent_amount: usize) -> String {
        let mut indent = String::new();
        for _ in 0..indent_amount {
            indent.push_str(" ");
        }
        indent
    }

    pub fn compute_indent(indent_amount: usize) -> String {
        let mut indent = String::new();
        for _ in 0..indent_amount {
            indent.push_str(&PAD_CHAR.white());
        }
        indent
    }
}

#[macro_use]
#[cfg(not(feature = "print-trace"))]
mod inner {
    pub struct TimerInfo;

    #[macro_export]
    macro_rules! start_timer {
        ($msg:expr) => {
            $crate::perf_trace::TimerInfo
        };
    }
    #[macro_export]
    macro_rules! add_to_trace {
        ($title:expr, $msg:expr) => {
            let _ = $msg;
        };
    }

    #[macro_export]
    macro_rules! end_timer {
        ($time:expr, $msg:expr) => {
            let _ = $msg;
            let _ = $time;
        };
        ($time:expr) => {
            let _ = $time;
        };
    }
}

mod tests {
    use super::*;

    #[test]
    fn print_start_end() {
        let start = start_timer!(|| "Hello");
        end_timer!(start);
    }

    #[test]
    fn print_add() {
        let start = start_timer!(|| "Hello");
        add_to_trace!(|| "HelloMsg", || "Hello, I\nAm\nA\nMessage");
        end_timer!(start);
    }
}
