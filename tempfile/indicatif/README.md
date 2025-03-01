# indicatif

[![Documentation](https://docs.rs/indicatif/badge.svg)](https://docs.rs/indicatif/)
[![Crates.io](https://img.shields.io/crates/v/indicatif.svg)](https://crates.io/crates/indicatif)
[![Build status](https://github.com/console-rs/indicatif/workflows/CI/badge.svg)](https://github.com/console-rs/indicatif/actions/workflows/rust.yml)
[![Chat](https://img.shields.io/discord/976380008299917365?logo=discord)](https://discord.gg/YHmNA3De4W)

A Rust library for indicating progress in command line applications to users.

This currently primarily provides progress bars and spinners as well as basic
color support, but there are bigger plans for the future of this!

## Examples

[examples/yarnish.rs](examples/yarnish.rs)
<img src="https://github.com/console-rs/indicatif/blob/main/screenshots/yarn.gif?raw=true">

[examples/download.rs](examples/download.rs)
<img src="https://github.com/console-rs/indicatif/blob/main/screenshots/download.gif?raw=true">

[examples/multi.rs](examples/multi.rs)
<img src="https://github.com/console-rs/indicatif/blob/main/screenshots/multi-progress.gif?raw=true">

[examples/single.rs](examples/single.rs)
<img src="https://github.com/console-rs/indicatif/blob/main/screenshots/single.gif?raw=true">

## Integrations

You can use [indicatif-log-bridge](https://crates.io/crates/indicatif-log-bridge) to integrate with the
[log crate](https://crates.io/crates/log) and avoid having both fight for your terminal.

You can use [tracing-indicatif](https://crates.io/crates/tracing-indicatif) to integrate with the
[tracing crate](https://crates.io/crates/tracing) with automatic progress bar management
for active tracing spans, as well as ensure that tracing
log events do not interfere with active progress bars.
