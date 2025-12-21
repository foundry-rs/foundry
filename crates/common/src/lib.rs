//! # foundry-common
//!
//! Common utilities for building and using foundry's tools.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[expect(unused_extern_crates)] // Used by `ConsoleFmt`.
extern crate self as foundry_common;

#[macro_use]
extern crate tracing;

#[macro_use]
pub mod io;

pub use foundry_common_fmt as fmt;

pub mod abi;
pub mod calc;
pub mod comments;
pub mod compile;
pub mod constants;
pub mod contracts;
pub mod errors;
pub mod fs;
pub mod iter;
pub mod mapping_slots;
mod preprocessor;
pub mod provider;
pub mod retry;
pub mod selectors;
pub mod serde_helpers;
pub mod slot_identifier;
pub mod term;
pub mod traits;
pub mod transactions;
mod utils;
pub mod version;

pub use compile::Analysis;
pub use constants::*;
pub use contracts::*;
pub use io::{Shell, shell, stdin};
pub use traits::*;
pub use transactions::*;
pub use utils::*;
