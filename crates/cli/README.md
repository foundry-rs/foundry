# Foundry CLIs

The CLIs are written using [clap's](https://docs.rs/clap) [derive feature](https://docs.rs/clap/latest/clap/_derive).

## Installation

See [Installation](../../README.md#Installation).

## Usage

Read the [📖 Foundry Book][foundry-book]

## Debugging

Debug logs are printed with
[`tracing`](https://docs.rs/tracing/latest/tracing/). You can configure the
verbosity level via the
[`RUST_LOG`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/fmt/index.html#filtering-events-with-environment-variables)
environment variable, on a per package level,
e.g.:`RUST_LOG=forge,foundry_evm forge test`

[foundry-book]: https://book.getfoundry.sh
