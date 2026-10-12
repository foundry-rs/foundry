//! # foundry-evm
//!
//! Foundry execution sessions and optional legacy engine integration.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "revm")]
#[macro_use]
extern crate tracing;

pub mod ethereum;
#[cfg(feature = "revm")]
pub mod executors;
#[cfg(feature = "revm")]
pub mod inspectors;

pub use foundry_evm_core as core;
#[cfg(feature = "revm")]
pub use foundry_evm_core::{EvmEnv, FoundryInspectorExt, InspectorExt, backend};
pub use foundry_evm_core::{constants, decode, fork, hardfork, opts, utils};
#[cfg(feature = "revm")]
pub use foundry_evm_coverage as coverage;
#[cfg(feature = "revm")]
pub use foundry_evm_fuzz as fuzz;
pub use foundry_evm_hardforks as hardforks;
pub use foundry_evm_traces as traces;

// TODO: We should probably remove these, but it's a pretty big breaking change.
#[doc(hidden)]
#[cfg(feature = "revm")]
pub use revm;
