## Debugging Foundry tools

This is a working document intended to outline some commands contributors can use to debug various parts of Foundry.

### Logs

All crates use [tracing](https://docs.rs/tracing/latest/tracing/) for logging. A console formatter is installed in each binary (`cast`, `forge`, `anvil`).

By setting `RUST_LOG=<filter>` you can get a lot more info out of Forge and Cast. For example, running Forge with `RUST_LOG=forge` will emit all logs of the `cli` crate, same for Cast.

The most basic valid filter is a log level, of which these are valid:

-   `error`
-   `warn`
-   `info`
-   `debug`
-   `trace`

Filters are explained in detail in the [`env_logger` crate docs](https://docs.rs/env_logger).

### Compiler input and output

You can get the compiler input JSON and output JSON by passing the `--build-info` flag.
This will create two files: one for the input and one for the output.
