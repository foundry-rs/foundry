//! # foundry-common
//!
//! Common utilities for building and using foundry's tools.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[allow(unused_extern_crates)] // Used by `ConsoleFmt`.
extern crate self as foundry_common;

#[macro_use]
extern crate tracing;

pub use foundry_common_fmt as fmt;

pub mod abi;
pub mod calc;
pub mod compile;
pub mod constants;
pub mod contracts;
pub mod ens;
pub mod errors;
pub mod evm;
pub mod fs;
pub mod provider;
pub mod retry;
pub mod selectors;
pub mod serde_helpers;
pub mod shell;
pub mod term;
pub mod traits;
pub mod transactions;
mod utils;

pub use constants::*;
pub use contracts::*;
pub use traits::*;
pub use transactions::*;
pub use utils::*;
