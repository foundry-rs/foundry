# <h1 align="center"> foundry </h1>

**Foundry is a blazing fast, portable and modular toolkit for Ethereum
application development written in Rust.**

![Github Actions](https://github.com/gakonst/foundry/workflows/Tests/badge.svg)
[![Telegram Chat](https://img.shields.io/endpoint?color=neon&style=flat-square&url=https%3A%2F%2Ftg.sumanjay.workers.dev%2Ffoundry_rs)](https://t.me/foundry_rs)
[![Crates.io][crates-badge]][crates-url]

[crates-badge]: https://img.shields.io/crates/v/foundry.svg
[crates-url]: https://crates.io/crates/foundry

## Features

1. Fast & flexible compilation pipeline:
   1. Automatic Solidity compiler version detection & installation (under
      `~/.svm`)
   1. Incremental compilation & caching: Only files changed are re-compiled
   1. Parallel compilation
   1. Non-standard directory structures support (e.g. can build
      [Hardhat repos](https://twitter.com/gakonst/status/1461289225337421829))
1. Tests are written in Solidity (like in DappTools)
1. Fast fuzz Tests with shrinking of inputs & printing of counter-examples
   1. These tests are ~3x faster than DappTools'
1. Fast remote RPC forking mode leveraging Rust's async infrastructure like
   tokio
1. Flexible debug logging:
   1. Dapptools-style, using `DsTest`'s emitted logs
   1. Hardhat-style, using the popular `console.sol` contract
1. Portable (7MB) & easy to install statically linked binary without requiring
   Nix or any other package manager
1. Abstracted over EVM implementations (currently supported: Sputnik, EvmOdin)

## Future Features

### Dapptools feature parity

Over the next months, we intend to add the following features which are
available in upstream dapptools:

1. Stack Traces
1. Symbolic EVM: The holy grail of testing, symbolically executed EVM allows
1. Invariant Tests
1. Interactive Debugger
1. Code coverage
1. Gas snapshots

### Unique features?

We also intend to add features which are not available in dapptools:

1. Faster tests with parallel EVM execution that produces state diffs instead of
   modifying the state
1. Improved UX for assertions:
   1. Check revert error or reason on a Solidity call
   1. Check that an event was emitted with expected arguments
1. Support more EVM backends (revm, geth's evm, hevm etc.) & benchmark
   performance across them
1. Declarative deployment system based on a config file
1. Formatting & Linting powered by [Solang]()
   1. `dapp fmt`, an automatic code formatter according to standard rules (like
      `prettier-plugin-solidity`)
   1. `dapp lint` a linter + static analyzer. think of this as `solhint` +
      slither + others.
1. Flamegraphs for gas profiling

## How Fast?

Forge is quite fast at both compiling (leveraging the ethers-solc package) and
testing.

Some benchmarks:

| Project   | Forge | DappTools |
| --------- | ----- | --------- |
| Header    | Title |
| Paragraph | Text  |

It also works with "non-standard" directory structures (i.e. contracts not in
`src/`, libraries not in `lib/`). When
[tested](https://twitter.com/gakonst/status/1461289225337421829) with
[`openzeppelin-contracts`](https://github.com/OpenZeppelin/openzeppelin-contracts),
Hardhat compilation took 15.244s, whereas Forge took 9.449 (~4s cached)

## Installing

We have not published a release yet. Until we do, please use the command below.
Because our dependencies may not be stable, do not forget the `--locked`
parameter, which will force the installer to use the checked in `Cargo.lock`
file.

```
cargo install --git https://github.com/gakonst/foundry --locked
```

Alternatively, clone the repository and run: `cargo build --release`

## Contributing

### Directory structure

This repository contains several Rust crates:

- [`forge`](forge): Library for building and testing a Solidity repository.
- [`cast`](cast): Library for interacting with a live Ethereum JSON-RPC
  compatible node, or for parsing data.
- [`cli`](cli): Command line interfaces to `cast` and `forge`.
- [`evm-adapters`](evm-adapters): Unified layer of abstraction over multiple EVM
  types. Currently supported EVMs:
  [Sputnik](https://github.com/rust-blockchain/evm/),
  [Evmodin](https://github.com/vorot93/evmodin).
- [`utils`](utils): Utilities for parsing ABI data, will eventually be
  upstreamed to [ethers-rs](https://github.com/gakonst/ethers-rs/).

The minimum supported rust version is 1.51.

### Rust Toolchain

We use the stable Rust toolchain. Install by running:
`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`.

#### Minimum Supported Rust Version

The current minimum supported Rust version is
`rustc 1.54.0 (a178d0322 2021-07-26)`.

### Testing

```
cargo check
cargo test
cargo doc --open
```

### Formatting

```
cargo +nightly fmt
cargo clippy
```

## Getting Help

First, see if the answer to your question can be found in the API documentation.
If the answer is not there, try opening an
[issue](https://github.com/gakonst/foundry/issues/new) with the question.

Join the [foundry telegram](https://t.me/foundry_rs) to chat with the community!

## Acknowledgements

- Foundry is a clean-room rewrite of the testing framework
  [dapptools](https://github.com/dapphub/dapptools). None of this would have
  been possible without the DappHub team's work over the eyars
- [Matthias Seitz](https://twitter.com/mattsse_): Created
  [ethers-solc](https://github.com/gakonst/ethers-rs/tree/master/ethers-solc/)
  which is the backbone of our compilation pipeline, as well as countless
  contributions to ethers, in particular the `abigen` macros.
- [Rohit Narunkar](https://twitter.com/rohitnarurkar): Created the Rust Solidity
  version manager [svm-rs](https://github.com/roynalnaruto/svm-rs) which we use
  to auto-detect and manage multiple Solidity versions
- All the other contributors to the ethers-rs & foundry repositories and
  chatrooms.
