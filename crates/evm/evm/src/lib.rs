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
pub use foundry_evm_core::{
    Env, EnvMut, EvmEnv, InspectorExt, backend, constants, decode, fork, hardfork, opts, utils,
};
pub use foundry_evm_coverage as coverage;
pub use foundry_evm_fuzz as fuzz;
pub use foundry_evm_traces as traces;

// TODO: We should probably remove these, but it's a pretty big breaking change.
#[doc(hidden)]
pub use revm;
