//! # foundry-wallets
//!
//! Utilities for working with multiple signers.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

pub mod error;
pub mod multi_wallet;
pub mod raw_wallet;
pub mod utils;
pub mod wallet;
pub mod wallet_signer;

pub use multi_wallet::MultiWalletOpts;
pub use raw_wallet::RawWalletOpts;
pub use wallet::WalletOpts;
pub use wallet_signer::{PendingSigner, WalletSigner};
