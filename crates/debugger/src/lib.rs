//! Opens Foundry's execution traces in the SolDB debugger.
//!
//! [`DebuggerBuilder::run`] starts a SolDB session or writes a SolDB bundle.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
extern crate foundry_common;

mod builder;
pub use builder::DebuggerBuilder;

mod soldb;
