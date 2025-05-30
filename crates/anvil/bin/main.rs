//! The `anvil` CLI: a fast local Ethereum development node, akin to Hardhat Network, Tenderly.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use anvil::args::run;

#[global_allocator]
static ALLOC: foundry_cli::utils::Allocator = foundry_cli::utils::new_allocator();

fn main() {
    if let Err(err) = run() {
        let _ = foundry_common::sh_err!("{err:?}");
        std::process::exit(1);
    }
}
