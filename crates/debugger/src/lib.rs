//! # foundry-debugger
//!
//! Interactive Solidity TUI debugger and debugger data file dumper

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate foundry_common;

#[macro_use]
extern crate tracing;

mod op;

mod builder;
mod debugger;
mod file_dumper;
mod tui;

mod node;

pub use node::DebugNode;

pub use builder::DebuggerBuilder;
pub use debugger::Debugger;
pub use file_dumper::FileDumper;
pub use tui::{ExitReason, TUI};
