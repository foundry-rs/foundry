//! # foundry-debugger
//!
//! Interactive Solidity TUI debugger.

#![warn(unused_crate_dependencies, unreachable_pub)]

#[macro_use]
extern crate foundry_common;
#[macro_use]
extern crate tracing;

mod op;

mod tui;
pub use tui::{Debugger, DebuggerBuilder, ExitReason};
