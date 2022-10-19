//! Common utilities for building and using foundry's tools.
#![deny(missing_docs, unused_crate_dependencies)]

pub mod abi;
pub mod calc;
pub mod clap_helpers;
pub mod compile;
pub mod constants;
pub mod contracts;
pub mod errors;
pub mod evm;
pub mod fmt;
pub mod fs;
pub mod provider;
pub mod selectors;
pub mod shell;
pub use provider::*;
pub mod term;
pub mod traits;
pub use constants::*;
pub use contracts::*;
pub use traits::*;
