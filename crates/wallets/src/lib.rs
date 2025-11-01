//! # foundry-wallets
//!
//! Utilities for working with multiple signers.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
extern crate foundry_common;

#[macro_use]
extern crate tracing;

pub mod error;
pub mod opts;
pub mod signer;
pub mod utils;
pub mod wallet_browser;
pub mod wallet_multi;
pub mod wallet_raw;

pub use opts::WalletOpts;
pub use signer::{PendingSigner, WalletSigner};
pub use wallet_multi::MultiWalletOpts;
pub use wallet_raw::RawWalletOpts;

#[cfg(feature = "aws-kms")]
use aws_config as _;
