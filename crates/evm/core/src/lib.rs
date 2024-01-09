//! # foundry-evm-core
//!
//! Core EVM abstractions.

#![warn(unused_crate_dependencies)]

#[macro_use]
extern crate tracing;

mod ic;

pub mod abi;
pub mod backend;
pub mod constants;
pub mod debug;
pub mod decode;
pub mod fork;
pub mod opts;
pub mod snapshot;
pub mod utils;
