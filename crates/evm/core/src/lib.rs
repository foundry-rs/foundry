//! # foundry-evm-core
//!
//! Generic execution, environment, fork, backend, and state abstractions shared by Foundry tools.
//!
//! All execution runs on [evm2](https://github.com/alloy-rs/evm2). [`evm::FoundryEvmNetwork`] binds
//! an Alloy network to an [`evm::FoundryEvmFactory`], which owns the network's evm2 type family and
//! builds Foundry-configured EVMs, while `foundry-evm-networks` owns runtime family selection.
//! Keeping those responsibilities separate allows one compiled binary to dispatch to different
//! execution families at runtime. [`backend::Backend`] layers RPC forks, persistent accounts,
//! snapshots and cheatcode policy on top; see its module documentation for the execution and
//! inspector contract.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

use alloy_primitives::{Address, map::HashMap};

#[cfg(feature = "optimism")]
use op_alloy_rpc_types as _;

/// Map keyed by breakpoints char to their location (contract address, pc)
pub type Breakpoints = HashMap<char, (Address, usize)>;

#[macro_use]
extern crate tracing;

pub mod abi {
    pub use foundry_cheatcodes_spec::Vm;
    pub use foundry_evm_abi::*;
}

pub mod backend;
pub mod buffer;
pub mod bytecode;
pub mod constants;
pub mod decode;
pub mod eip2935;
pub mod evm;
pub mod fork;
pub mod hardfork;
pub mod ic;
pub mod opts;
pub mod precompiles;
pub mod state_snapshot;
pub mod tempo;
pub mod utils;
