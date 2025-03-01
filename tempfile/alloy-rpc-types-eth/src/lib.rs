#![doc = include_str!("../README.md")]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/alloy.jpg",
    html_favicon_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/favicon.ico"
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![cfg_attr(not(any(test, feature = "std")), no_std)]

#[macro_use]
#[allow(unused_imports)]
extern crate alloc;

pub use alloy_eips::eip4895::{Withdrawal, Withdrawals};

pub use alloy_network_primitives::{
    BlockTransactionHashes, BlockTransactions, BlockTransactionsKind,
};

mod account;
pub use account::*;

mod block;
pub use block::*;

mod call;
pub use call::{Bundle, EthCallResponse, StateContext, TransactionIndex};

pub mod error;

mod fee;
pub use fee::{FeeHistory, TxGasAndReward};

mod filter;
pub use filter::*;

mod index;
pub use index::Index;

mod log;
pub use log::*;

#[cfg(feature = "serde")]
pub mod pubsub;

mod raw_log;
pub use raw_log::{logs_bloom, Log as RawLog};

pub mod state;

mod syncing;
pub use syncing::*;

pub mod transaction;
pub use transaction::*;

mod work;
pub use work::Work;

/// This module provides implementations for EIP-4337.
pub mod erc4337;
pub use erc4337::{
    PackedUserOperation, SendUserOperation, SendUserOperationResponse, UserOperation,
    UserOperationGasEstimation, UserOperationReceipt,
};

pub mod simulate;
