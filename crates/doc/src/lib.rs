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
#[macro_use]
extern crate tracing;

mod builder;
pub use builder::DocBuilder;

mod document;
pub use document::Document;

mod helpers;

mod parser;
pub use parser::{
    error, Comment, CommentTag, Comments, CommentsRef, ParseItem, ParseSource, Parser,
};

mod preprocessor;
pub use preprocessor::*;

mod writer;
pub use writer::{AsDoc, AsDocResult, BufWriter, Markdown};

pub use mdbook;
