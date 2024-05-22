//! Common utilities for building and using foundry's tools.

#![warn(missing_docs, unused_crate_dependencies)]

extern crate self as foundry_common;

#[macro_use]
extern crate tracing;

pub mod abi;
pub mod calc;
pub mod compile;
pub mod constants;
pub mod contracts;
pub mod ens;
pub mod errors;
pub mod evm;
pub mod fmt;
pub mod fs;
pub mod glob;
pub mod provider;
pub mod retry;
pub mod selectors;
pub mod serde_helpers;
pub mod shell;
pub mod term;
pub mod traits;
pub mod transactions;

pub use constants::*;
pub use contracts::*;
pub use traits::*;
pub use transactions::*;

/// Block on a future using the current tokio runtime on the current thread.
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    block_on_handle(&tokio::runtime::Handle::current(), future)
}

/// Block on a future using the current tokio runtime on the current thread with the given handle.
pub fn block_on_handle<F: std::future::Future>(
    handle: &tokio::runtime::Handle,
    future: F,
) -> F::Output {
    tokio::task::block_in_place(|| handle.block_on(future))
}
