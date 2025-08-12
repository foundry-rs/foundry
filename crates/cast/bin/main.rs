//! The `cast` CLI: a Swiss Army knife for interacting with EVM smart contracts, sending
//! transactions and getting chain data.

use cast::args::run;

#[global_allocator]
static ALLOC: foundry_cli::utils::Allocator = foundry_cli::utils::new_allocator();

fn main() {
    if let Err(err) = run() {
        let _ = foundry_common::sh_err!("{err:?}");
        std::process::exit(1);
    }
}
