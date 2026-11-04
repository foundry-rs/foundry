//! # foundry-cli
//!
//! Common CLI utilities.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
extern crate foundry_common;

#[macro_use]
extern crate tracing;

// Unused. Kept to align dependency features across package selections; see Cargo.toml.
use ahash as _;
use hashbrown as _;
use itertools014 as _;
use num_bigint as _;
use ruint as _;
#[cfg(unix)]
use rustix as _;

pub mod clap;
pub mod handler;
pub mod install;
pub mod json;
pub mod lockfile;
pub mod opts;
pub mod utils;

#[cfg(feature = "tracy")]
tracing_tracy::client::register_demangler!();
