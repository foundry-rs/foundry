//! Forge is a fast and flexible Ethereum testing framework.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
extern crate foundry_common;

#[macro_use]
extern crate tracing;

// Required for optional features (aws-kms, gcp-kms, turnkey)
#[cfg(any(feature = "aws-kms", feature = "gcp-kms", feature = "turnkey"))]
use foundry_wallets as _;

pub mod args;
pub mod cmd;
pub mod opts;

pub mod coverage;

pub mod brutalizer;

pub mod gas_report;

#[cfg(feature = "revm")]
pub mod multi_runner;
#[cfg(feature = "revm")]
pub use multi_runner::{MultiContractRunner, MultiContractRunnerBuilder};

#[cfg(feature = "revm")]
pub mod mutation;

pub mod workspace;

#[cfg(feature = "revm")]
mod runner;
#[cfg(feature = "revm")]
pub use runner::ContractRunner;

mod ethereum_runner;
mod invariant_failure;
pub mod test_artifacts;
mod test_config;
pub mod test_options;

#[cfg(feature = "revm")]
mod progress;
pub mod result;
#[cfg(feature = "revm")]
mod symbolic_minimizer;
#[cfg(feature = "revm")]
mod symbolic_regression;

// TODO: remove
pub use foundry_common::traits::TestFilter;
pub use foundry_evm::*;

pub use foundry_cli::lockfile::{DepIdentifier, DepMap, FOUNDRY_LOCK, Lockfile};
