//! # foundry-debugger
//!
//! Interactive Solidity TUI debugger.

#![warn(unused_crate_dependencies)]

#[macro_use]
extern crate tracing;

mod op;

mod tui;
pub use tui::{Debugger, DebuggerBuilder, ExitReason};
