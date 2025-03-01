[![Crate release version](https://badgen.net/crates/v/clearscreen)](https://crates.io/crates/clearscreen)
[![Crate license: Apache 2.0 or MIT](https://badgen.net/badge/license/Apache%202.0%20or%20MIT)][copyright]
[![CI status on main branch](https://github.com/watchexec/clearscreen/actions/workflows/tests.yml/badge.svg)](https://github.com/watchexec/clearscreen/actions/workflows/main.yml)

# ClearScreen

_Cross-platform terminal screen clearing library._

- **[API documentation][docs]**.
- [Dual-licensed][copyright] with Apache 2.0 and MIT.
- Minimum Supported Rust Version: 1.79.0.
  - Only the last five stable versions are supported.
  - MSRV increases beyond that range at publish time will not incur major version bumps.

[copyright]: ./COPYRIGHT
[docs]: https://docs.rs/clearscreen

Tested with and tweaked for over 80 different terminals, multiplexers, SSH clients.
See my research notes in the [TERMINALS.md](./TERMINALS.md) file.

## Quick start

```toml
[dependencies]
clearscreen = "4.0.1"
```

```rust
clearscreen::clear().unwrap();
```
