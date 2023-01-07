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
mod document;
mod parser;
mod preprocessor;
mod writer;

/// The documentation builder.
pub use builder::DocBuilder;

/// The document output.
pub use document::Document;

/// Solidity parser and related output items.
pub use parser::{error, ParseItem, ParseSource, Parser};

/// Preprocessors.
pub use preprocessor::*;

/// Traits for formatting items into doc output.
pub use writer::{AsCode, AsDoc, AsDocResult, BufWriter, Markdown};
