# Foundry Compilers
| [Docs](https://docs.rs/foundry-compilers/latest/foundry_compilers/) |

Originally part of [`ethers-rs`] as `ethers-solc`, Foundry Compilers is the compilation backend for [Foundry](https://github.com/foundry-rs/foundry).

[`ethers-rs`]'s `ethers-solc` is considered to be in maintenance mode, and any fixes to it will also be reflected on Foundry Compilers. No action is currently needed from devs, although we heavily recommend using Foundry Compilers instead of `ethers-solc`.

[`ethers-rs`]: https://github.com/gakonst/ethers-rs

[![Build Status][actions-badge]][actions-url]
[![Telegram chat][telegram-badge]][telegram-url]

[actions-badge]: https://img.shields.io/github/actions/workflow/status/foundry-rs/compilers/ci.yml?branch=main&style=for-the-badge
[actions-url]: https://github.com/foundry-rs/compilers/actions?query=branch%3Amain
[telegram-badge]: https://img.shields.io/endpoint?color=neon&style=for-the-badge&url=https%3A%2F%2Ftg.sumanjay.workers.dev%2Ffoundry_rs
[telegram-url]: https://t.me/foundry_rs

## Supported Rust Versions

<!--
When updating this, also update:
- clippy.toml
- Cargo.toml
- .github/workflows/ci.yml
-->

Foundry Compilers will keep a rolling MSRV (minimum supported rust version) policy of **at
least** 6 months. When increasing the MSRV, the new Rust version must have been
released at least six months ago. The current MSRV is 1.83.0.

Note that the MSRV is not increased automatically, and only as part of a minor
release.

## Contributing

Thanks for your help in improving the project! We are so happy to have you! We have
[a contributing guide](./CONTRIBUTING.md) to help you get involved in the
Foundry Compilers project.

Pull requests will not be merged unless CI passes, so please ensure that your
contribution follows the linting rules and passes clippy.

## Overview

To install, simply add `foundry-compilers` to your cargo dependencies.

```toml
[dependencies]
foundry-compilers = "0.10.1"
```

Example usage:

```rust,ignore
use foundry_compilers::{Project, ProjectPathsConfig};
use std::path::Path;

// configure the project with all its paths, solc, cache etc.
let project = Project::builder()
    .paths(ProjectPathsConfig::hardhat(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap())
    .build(Default::default())
    .unwrap();
let output = project.compile().unwrap();

// Tell Cargo that if a source file changes, to rerun this build script.
project.rerun_if_sources_changed();
```
