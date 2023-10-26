//! # foundry-evm
//!
//! EVM executor and inspector implementations.

#![warn(unreachable_pub, unused_crate_dependencies, rust_2018_idioms)]

#[macro_use]
extern crate tracing;

pub mod executors;
pub mod inspectors;

pub use foundry_evm_coverage as coverage;
pub use foundry_evm_executors::{backend, constants, debug, decode, fork, opts, utils};
pub use foundry_evm_fuzz as fuzz;
pub use foundry_evm_traces as traces;

#[doc(hidden)]
pub use {hashbrown, revm};
