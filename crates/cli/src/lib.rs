//! # foundry-cli
//!
//! Common CLI utilities.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

pub mod handler;
pub mod opts;
pub mod stdin;
pub mod utils;
