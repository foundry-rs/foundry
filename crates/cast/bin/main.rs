//! The `cast` CLI: a Swiss Army knife for interacting with EVM smart contracts, sending
//! transactions and getting chain data.

use cast::args::run;

#[global_allocator]
static ALLOC: foundry_cli::utils::Allocator = foundry_cli::utils::new_allocator();

fn main() {
    if let Err(err) = run() {
        if foundry_cli::is_machine() {
            foundry_cli::machine::report_machine_error(&err);
            std::process::exit(foundry_cli::ExitCode::from(&err).to_i32());
        }
        let _ = foundry_common::sh_err!("{err:?}");
        std::process::exit(1);
    }
}
