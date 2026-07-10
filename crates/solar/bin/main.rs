//! The `solar` CLI: a Solidity compiler.

use solar_cli::utils;

#[global_allocator]
static ALLOC: utils::Allocator = utils::new_allocator();

pub use solar_cli::main;
