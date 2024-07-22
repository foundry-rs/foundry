//! # foundry-debugger
//!
//! Interactive Solidity TUI debugger.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

mod op;

mod tui;
pub use tui::{Debugger, DebuggerBuilder, ExitReason};

mod node;
pub use node::DebugNode;
