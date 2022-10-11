#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![warn(unused_extern_crates)]
#![forbid(unsafe_code)]
#![forbid(where_clauses_object_safety)]

/// Chisel Environment Module
pub mod session;

/// A wrapper around [solang_parser](solang_parser::parse) parser to generate
/// [SourceUnit](solang_parser::ast::SourceUnit)s from a solidity source code strings
pub mod parser;

/// REPL command dispatcher.
pub mod dispatcher;

/// The runner
pub mod runner;

/// A Solidity Highlighter module
pub mod sol_highlighter;

/// Session Source
pub mod generator;

/// Re-export a prelude of relevant chisel items
pub mod prelude {
    pub use crate::{dispatcher::*, generator::*, session::*, runner::*, sol_highlighter::*};
}
