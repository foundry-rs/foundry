//! Common utilities for building and using foundry's tools.

#![warn(missing_docs, unused_crate_dependencies)]

#[macro_use]
extern crate tracing;
extern crate self as foundry_common;

#[macro_use]
pub mod io;

pub mod abi;
pub mod calc;
pub mod compile;
pub mod constants;
pub mod contracts;
pub mod errors;
pub mod evm;
pub mod fmt;
pub mod fs;
pub mod glob;
pub mod provider;
pub mod retry;
pub mod rpc;
pub mod runtime_client;
pub mod selectors;
pub mod term;
pub mod traits;
pub mod transactions;
pub mod types;
pub mod units;

pub use constants::*;
pub use contracts::*;
pub use provider::*;
pub use traits::*;
pub use transactions::*;

pub use io::{shell, stdin, Shell};
