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
pub mod handler;
pub mod opts;
pub mod utils;

#[cfg(feature = "tracy")]
tracing_tracy::client::register_demangler!();
