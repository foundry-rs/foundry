#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![warn(unused_extern_crates)]
#![forbid(unsafe_code)]
#![forbid(where_clauses_object_safety)]

/// Chisel Environment Module
pub mod env;

/// REPL command dispatcher.
pub mod cmd;

/// A Solidity Highlighter module
pub mod sol_highlighter;

/// Session Source
pub mod source;

/// Re-export a prelude of relevant chisel items
pub mod prelude {
    pub use crate::{
        cmd::*,
        env::*,
        source::*,
        sol_highlighter::*
    };
}
