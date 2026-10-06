//! The `forge` CLI: build, test, fuzz, debug and deploy Solidity contracts, like Hardhat, Brownie,
//! Ape.

#![cfg_attr(
    target_os = "macos",
    allow(
        linker_messages,
        reason = "Apple ld cannot encode Forge's large unwind table in its compact format"
    )
)]

use forge::args::run;

#[global_allocator]
static ALLOC: foundry_cli::utils::Allocator = foundry_cli::utils::new_allocator();

fn main() {
    if let Err(err) = run() {
        let _ = foundry_common::sh_err!("{err:?}");
        std::process::exit(1);
    }
}
