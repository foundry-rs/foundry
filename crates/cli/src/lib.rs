//! # foundry-cli
//!
//! Common CLI utilities.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
extern crate foundry_common;

#[macro_use]
extern crate tracing;

pub mod clap;
pub mod diagnostic;
pub mod exit_code;
pub mod handler;
pub mod introspect;
pub mod json;
pub mod machine;
pub mod opts;
pub mod utils;

pub use exit_code::ExitCode;
pub use machine::{check_machine, is_machine, parse_or_exit};

#[cfg(feature = "tracy")]
tracing_tracy::client::register_demangler!();
