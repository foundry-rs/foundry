#![warn(missing_debug_implementations, missing_docs, unreachable_pub)]
#![deny(unused_must_use, rust_2018_idioms)]
#![doc(test(
    no_crate_inject,
    attr(deny(warnings, rust_2018_idioms), allow(dead_code, unused_variables))
))]

//! The module for generating Solidity documentation.
//!
//! See [DocBuilder]

pub use builder::DocBuilder;
pub use config::DocConfig;

mod as_code;
mod builder;
mod config;
mod format;
mod helpers;
mod macros;
mod output;
mod parser;
