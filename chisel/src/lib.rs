#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![warn(unused_extern_crates)]
#![forbid(unsafe_code)]
#![forbid(where_clauses_object_safety)]

/// Chisel Environment Module
pub mod env;

/// REPL command dispatcher.
pub mod cmd;

/// The main Chisel Module
pub mod chisel;

/// A Solidity Highlighter module
pub mod sol_highlighter;

/// Re-export a prelude of relevant chisel items
pub mod prelude {
    pub use crate::{
        env::*,
        chisel::*,
        sol_highlighter::*,
    };
}