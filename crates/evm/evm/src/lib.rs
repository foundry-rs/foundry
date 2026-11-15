//! # foundry-evm
//!
//! Main Foundry EVM backend abstractions.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
extern crate tracing;

pub mod executors;
pub mod inspectors;

pub use foundry_evm_core as core;
pub use foundry_evm_core::{backend, constants, decode, evm::EvmEnv, fork, hardfork, opts, utils};
pub use foundry_evm_coverage as coverage;
pub use foundry_evm_fuzz as fuzz;
pub use foundry_evm_hardforks as hardforks;
pub use foundry_evm_traces as traces;

/// Stack size of the threads that run EVM executions.
///
/// evm2 executes every call frame as a native recursion, so a call at the maximum depth of 1024
/// needs several MiB of stack in debug builds, more than the default 2 MiB.
pub const EVM_THREAD_STACK_SIZE: usize = 32 * 1024 * 1024;
