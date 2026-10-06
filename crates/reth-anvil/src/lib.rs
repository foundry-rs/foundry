//! Reth-anvil is a local Ethereum development node built on the reth SDK.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod api;
mod block_env;
mod evm;
mod impersonation;
mod mining;
mod node;
mod pool;
mod time;

pub use api::{AnvilApiServer, AnvilRpc};
pub use node::{RethAnvilConfig, RethAnvilHandle, launch};
