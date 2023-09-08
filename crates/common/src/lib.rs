//! Common utilities for building and using foundry's tools.

#![warn(missing_docs, unused_crate_dependencies)]

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
pub mod runtime_client;
pub mod selectors;
pub mod term;
pub mod traits;
pub mod transactions;

pub use constants::*;
pub use contracts::*;
pub use provider::*;
pub use traits::*;
pub use transactions::*;

pub use io::{shell, stdin, Shell};
