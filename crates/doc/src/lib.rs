//! The module for generating Solidity documentation.
//!
//! See [DocBuilder]

#![warn(missing_debug_implementations, missing_docs, unreachable_pub, unused_crate_dependencies)]
#![deny(unused_must_use, rust_2018_idioms)]
#![doc(test(
    no_crate_inject,
    attr(deny(warnings, rust_2018_idioms), allow(dead_code, unused_variables))
))]

#[macro_use]
extern crate foundry_common;

mod builder;
mod document;
mod helpers;
mod parser;
mod preprocessor;
mod server;
mod writer;

/// The documentation builder.
pub use builder::DocBuilder;

/// The documentation server.
pub use server::Server;

/// The document output.
pub use document::Document;

/// Solidity parser and related output items.
pub use parser::{
    error, Comment, CommentTag, Comments, CommentsRef, ParseItem, ParseSource, Parser,
};

/// Preprocessors.
pub use preprocessor::*;

/// Traits for formatting items into doc output.
pub use writer::{AsDoc, AsDocResult, BufWriter, Markdown};
