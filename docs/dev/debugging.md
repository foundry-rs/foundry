# Debugging

This is a working document intended to outline some commands contributors can use to debug various parts of Foundry.

## Logs

All crates use [tracing](https://docs.rs/tracing/latest/tracing/) for logging. A console formatter is installed in each binary (`cast`, `forge`, `anvil`).

By setting `RUST_LOG=<filter>` you can get a lot more info out of Forge and Cast. For example, running Forge with `RUST_LOG=forge` will emit all logs from the `forge` package, same for Cast with `RUST_LOG=cast`.

The most basic valid filter is a log level, of which these are valid:

- `error`
- `warn`
- `info`
- `debug`
- `trace`

Filters are explained in detail in the [`tracing-subscriber` crate docs](https://docs.rs/tracing-subscriber).

You can also use the `dbg!` macro from Rust's standard library.
