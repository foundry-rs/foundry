#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![warn(unused_extern_crates)]
#![forbid(unsafe_code)]
#![forbid(where_clauses_object_safety)]

/// Chisel Environment Module
pub mod session;

/// REPL command dispatcher.
pub mod dispatcher;

/// Session Source
pub mod session_source;

/// The runner
pub mod runner;

/// The executor
pub mod executor;

/// A Solidity Helper module
pub mod solidity_helper;

/// Re-export a prelude of relevant chisel items
pub mod prelude {
    pub use crate::{dispatcher::*, runner::*, session::*, session_source::*, solidity_helper::*};
}
