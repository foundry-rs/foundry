# Debugging foundry tools

All crates use [tracing](https://docs.rs/tracing/latest/tracing/) for logging. An console formatter is installed in each binary (`cast`, `forge`, `anvil`).

Logging output is enabled via `RUST_LOG` var. Running `forge` with `RUST_LOG=forge` will emit all logs of the `cli` crate, same for `cast`.
