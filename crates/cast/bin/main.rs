//! The `cast` CLI: a Swiss Army knife for interacting with EVM smart contracts, sending
//! transactions and getting chain data.

use cast::args::run;
use foundry_cli::json::{JsonEnvelope, JsonMessage, print_json};

#[global_allocator]
static ALLOC: foundry_cli::utils::Allocator = foundry_cli::utils::new_allocator();

fn main() {
    if let Err(err) = run() {
        if foundry_cli::is_machine() {
            foundry_cli::machine::report_machine_error(&err);
            std::process::exit(foundry_cli::ExitCode::GenericError.to_i32());
        }
        if foundry_common::shell::is_json() {
            // Collect the full error chain into structured error entries.
            let errors = err
                .chain()
                .enumerate()
                .map(|(i, e)| {
                    if i == 0 {
                        JsonMessage::error("cast.error", e.to_string())
                    } else {
                        JsonMessage::error("cast.error.context", e.to_string())
                    }
                })
                .collect::<Vec<_>>();
            let envelope = JsonEnvelope::<()>::failure(errors);
            let _ = print_json(&envelope);
        } else {
            let _ = foundry_common::sh_err!("{err:?}");
        }
        std::process::exit(1);
    }
}
