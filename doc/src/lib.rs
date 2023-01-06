#![warn(missing_debug_implementations, missing_docs, unreachable_pub)]
#![deny(unused_must_use, rust_2018_idioms)]
#![doc(test(
    no_crate_inject,
    attr(deny(warnings, rust_2018_idioms), allow(dead_code, unused_variables))
))]

//! The module for generating Solidity documentation.
//!
//! See [DocBuilder]

mod builder;
mod format;
mod helpers;
mod output;
mod parser;
mod writer;

/// The documentation builder.
pub use builder::DocBuilder;

/// Solidity parser and related output items.
pub use parser::{error, ParseItem, ParseSource, Parser};

/// Traits for formatting items into doc output/
pub use format::{AsCode, AsDoc, AsDocResult};
