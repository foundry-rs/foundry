#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![warn(unused_extern_crates)]
#![forbid(unsafe_code)]
#![forbid(where_clauses_object_safety)]

use lazy_static::lazy_static;
use std::path::PathBuf;

lazy_static! {
    /// The path to `forge-std`'s `Script.sol` in `testdata`
    pub static ref SCRIPT_PATH: PathBuf =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../testdata/lib/forge-std/src/Script.sol");
}

/// Chisel Environment Module
pub mod session;

/// REPL input dispatcher module
pub mod dispatcher;

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
    pub use crate::{dispatcher::*, runner::*, session::*, session_source::*, solidity_helper::*};
}
