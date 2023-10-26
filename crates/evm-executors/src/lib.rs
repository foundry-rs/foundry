//! # foundry-evm-executors
//!
//! EVM executor abstractions, which can execute calls.
//!
//! Used for running tests, scripts, and interacting with the inner backend which holds the state.

#![warn(unused_crate_dependencies)]

#[macro_use]
extern crate tracing;

pub mod abi;
pub mod backend;
pub use backend::Backend;
pub mod constants;
pub mod debug;
pub mod decode;
pub mod fork;
pub mod opts;
pub mod snapshot;
pub mod utils;

pub use revm::primitives::State as StateChangeset;

// TODO: Remove these re-exports
pub use abi::{
    patch_hardhat_console_selector, HardhatConsoleCalls, CHEATCODE_ADDRESS, CONSOLE_ABI,
    HARDHAT_CONSOLE_ABI, HARDHAT_CONSOLE_ADDRESS,
};
pub use constants::*;
pub use utils::CallKind;
