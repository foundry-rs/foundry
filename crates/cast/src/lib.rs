//! Cast is a Swiss Army knife for interacting with Ethereum applications from the command line.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![recursion_limit = "256"]

#[macro_use]
extern crate foundry_common;
#[macro_use]
extern crate tracing;

#[cfg(feature = "optimism")]
use op_alloy_consensus as _;

pub use foundry_evm::*;

pub mod args;
pub mod cmd;
pub mod opts;
pub mod tempo;

pub mod base;
pub mod call_spec;
pub(crate) mod debug;
mod rlp_converter;
pub mod rpc_trace;
pub mod tx;
