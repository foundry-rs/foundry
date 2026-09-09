//! The `cast` CLI: a Swiss Army knife for interacting with EVM smart contracts, sending
//! transactions and getting chain data.

use cast::args::run;
use foundry_cli::json::{JsonEnvelope, JsonError, JsonMessage, print_json};

#[global_allocator]
static ALLOC: foundry_cli::utils::Allocator = foundry_cli::utils::new_allocator();

fn main() {
    if let Err(err) = run() {
        if foundry_common::shell::is_json() {
            if let Some(err) = err.downcast_ref::<JsonError>() {
                let envelope = JsonEnvelope::failure_with_data(&err.data, err.errors.clone());
                let _ = print_json(&envelope);
                std::process::exit(1);
            }
            // Collect the full error chain into structured error entries.
            let errors = err
                .chain()
                .enumerate()
                .map(|(i, e)| {
                    let code = if i == 0 { "cast.error" } else { "cast.error.context" };
                    JsonMessage::error(code, e.to_string())
                })
                .collect();
            let _ = print_json(&JsonEnvelope::<()>::failure(errors));
        } else {
            let _ = foundry_common::sh_err!("{err:?}");
        }
        std::process::exit(1);
    }
}
