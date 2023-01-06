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

mod as_code;
mod builder;
mod format;
mod helpers;
mod output;
mod parser;
mod writer;
