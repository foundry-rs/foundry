//! The `anvil` CLI: a fast local Ethereum development node, akin to Hardhat Network, Tenderly.

use anvil::args::run;

#[global_allocator]
static ALLOC: foundry_cli::utils::Allocator = foundry_cli::utils::new_allocator();

fn main() {
    if let Err(err) = run() {
        if foundry_cli::is_machine() {
            foundry_cli::machine::report_machine_error(&err);
            std::process::exit(foundry_cli::ExitCode::GenericError.to_i32());
        }
        let _ = foundry_common::sh_err!("{err:?}");
        std::process::exit(1);
    }
}
