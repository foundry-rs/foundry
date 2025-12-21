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

pub mod gas_report;

pub mod multi_runner;
pub use multi_runner::{MultiContractRunner, MultiContractRunnerBuilder};

mod runner;
pub use runner::ContractRunner;

mod progress;
pub mod result;

// TODO: remove
pub use foundry_common::traits::TestFilter;
pub use foundry_evm::*;

mod lockfile;
pub use lockfile::{DepIdentifier, DepMap, FOUNDRY_LOCK, Lockfile};
