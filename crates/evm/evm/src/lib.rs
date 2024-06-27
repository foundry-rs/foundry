//! # foundry-evm
//!
//! Main Foundry EVM backend abstractions.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

pub mod executors;
pub mod inspectors;

pub use foundry_evm_core::{backend, constants, decode, fork, opts, utils, InspectorExt};
pub use foundry_evm_coverage as coverage;
pub use foundry_evm_fuzz as fuzz;
pub use foundry_evm_traces as traces;

// TODO: We should probably remove these, but it's a pretty big breaking change.
#[doc(hidden)]
pub use revm;

#[doc(hidden)]
#[deprecated = "use `{hash_map, hash_set, HashMap, HashSet}` in `std::collections` or `revm::primitives` instead"]
pub mod hashbrown {
    pub use revm::primitives::{hash_map, hash_set, HashMap, HashSet};
}
