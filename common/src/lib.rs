//! Common utilities for building and using foundry's tools.

#![deny(missing_docs, unused_crate_dependencies)]

pub mod calc;
pub mod clap_helpers;
pub mod constants;
pub mod contracts;
pub mod errors;
pub mod evm;
pub mod fmt;
pub mod fs;
pub mod provider;
pub mod shell;
pub use provider::*;
pub mod traits;
pub use constants::*;
pub use contracts::*;
pub use traits::*;
