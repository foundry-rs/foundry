//! # foundry-debugger
//!
//! Interactive Solidity TUI debugger and debugger data file dumper

#![warn(unused_crate_dependencies, unreachable_pub)]

#[macro_use]
extern crate tracing;

mod op;

mod builder;
mod context;
mod debugger;
mod file_dumper;
mod tui;

pub use builder::DebuggerBuilder;
pub use debugger::Debugger;
pub use file_dumper::FileDumper;
pub use tui::{ExitReason, TUI};
