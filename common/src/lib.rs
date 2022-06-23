//! Common utilities for building and using foundry's tools.

#![deny(missing_docs, unsafe_code, unused_crate_dependencies)]

pub mod errors;
pub mod evm;
pub mod fs;

/// The dev chain-id, inherited from hardhat
pub const DEV_CHAIN_ID: u64 = 31337;
