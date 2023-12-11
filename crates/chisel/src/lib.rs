#![doc = include_str!("../README.md")]
#![allow(clippy::disallowed_macros)]
#![warn(missing_docs, unused_extern_crates)]

/// REPL input dispatcher module
pub mod dispatcher;

/// Builtin Chisel commands
pub mod cmd;

pub mod history;

/// Chisel Environment Module
pub mod session;

/// Chisel Session Source wrapper
pub mod session_source;

/// REPL contract runner
pub mod runner;

/// REPL contract executor
pub mod executor;

/// A Solidity Helper module for rustyline
pub mod solidity_helper;

/// Prelude of all chisel modules
pub mod prelude {
    pub use crate::{
        cmd::*, dispatcher::*, runner::*, session::*, session_source::*, solidity_helper::*,
    };
}
