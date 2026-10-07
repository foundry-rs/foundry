//! # foundry-debugger
//!
//! Interactive Solidity TUI debugger and debugger data file dumper

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
extern crate foundry_common;

#[macro_use]
extern crate tracing;

mod op;

mod builder;
mod debugger;
mod dump;
#[cfg(feature = "soldb")]
mod soldb;
mod tui;

mod node;

pub use node::DebugNode;

pub use builder::DebuggerBuilder;
pub use debugger::{Debugger, DebuggerFrontend, DebuggerLayout};
pub use tui::{ExitReason, TUI};
