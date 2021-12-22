# <h1 align="center"> foundry </h1>

![Github Actions](https://github.com/gakonst/foundry/workflows/Tests/badge.svg)
[![Telegram Chat](https://img.shields.io/endpoint?color=neon&style=flat-square&url=https%3A%2F%2Ftg.sumanjay.workers.dev%2Ffoundry_rs)](https://t.me/foundry_rs)
[![Crates.io][crates-badge]][crates-url]

[crates-badge]: https://img.shields.io/crates/v/foundry.svg
[crates-url]: https://crates.io/crates/foundry-rs

**Foundry is a blazing fast, portable and modular toolkit for Ethereum
application development written in Rust.**

Foundry consists of:

- [**Forge**](./forge): Ethereum testing framework (like Truffle, Hardhat and
  Dapptools).
- [**Cast**](./cast): Swiss army knife for interacting with EVM smart contracts,
  sending transactions and getting chain data.

![demo](./assets/demo.svg)

## Forge

```
cargo install --git https://github.com/gakonst/foundry --bin forge --locked
```

If you are on a x86/x86_64 Unix machine, you can also use `--features=solc-asm`
to enable Sha2 Assembly instructions, which further speedup the compilation
pipeline cache.

We also recommend using [forgeup](https://github.com/transmissions11/forgeup)
for managing various versions of Forge, so that you can easily test out bleeding
edge changes in open pull requests or forks from contributors.

More documentation can be found in the [forge package](./forge/README.md) and in
the [CLI README](./cli/README.md).

### Features

1. Fast & flexible compilation pipeline:
   1. Automatic Solidity compiler version detection & installation (under
      `~/.svm`)
   1. Incremental compilation & caching: Only changed files are re-compiled
   1. Parallel compilation
   1. Non-standard directory structures support (e.g. can build
      [Hardhat repos](https://twitter.com/gakonst/status/1461289225337421829))
1. Tests are written in Solidity (like in DappTools)
1. Fast fuzz Tests with shrinking of inputs & printing of counter-examples
1. Fast remote RPC forking mode leveraging Rust's async infrastructure like
   tokio
1. Flexible debug logging:
   1. Dapptools-style, using `DsTest`'s emitted logs
   1. Hardhat-style, using the popular `console.sol` contract
1. Portable (5-10MB) & easy to install statically linked binary without
   requiring Nix or any other package manager
1. Abstracted over EVM implementations (currently supported: Sputnik, EvmOdin)

### How Fast?

Forge is quite fast at both compiling (leveraging the
[ethers-solc](https://github.com/gakonst/ethers-rs/tree/master/ethers-solc/)
package) and testing.

Some benchmarks below:

| Project                                             | Forge | DappTools | Speedup |
| --------------------------------------------------- | ----- | --------- | ------- |
| [guni-lev](https://github.com/hexonaut/guni-lev/)   | 28.6s | 2m36s     | 5.45x   |
| [solmate](https://github.com/Rari-Capital/solmate/) | 6s    | 46s       | 7.66x   |
| [geb](https://github.com/reflexer-labs/geb)         | 11s   | 40s       | 3.63x   |
| [vaults](https://github.com/rari-capital/vaults)    | 1.4s  | 5.5s      | 3.9x    |

It also works with "non-standard" directory structures (i.e. contracts not in
`src/`, libraries not in `lib/`). When
[tested](https://twitter.com/gakonst/status/1461289225337421829) with
[`openzeppelin-contracts`](https://github.com/OpenZeppelin/openzeppelin-contracts),
Hardhat compilation took 15.244s, whereas Forge took 9.449 (~4s cached)

## Cast

Cast is a swiss army knife for interacting with Ethereum applications from the
command line.

```
cargo install --git https://github.com/gakonst/foundry --bin cast
// Get USDC's total supply
cast call 0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48 "totalSupply()(uint256)" --rpc-url <..your node url>
```

More documentation can be found in the [cast package](./cast/README.md).

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

We use the nightly toolchain for formatting and linting.

```
cargo +nightly fmt
cargo +nightly clippy --all-features -- -D warnings
```

## Getting Help

First, see if the answer to your question can be found in the API documentation.
If the answer is not there, try opening an
[issue](https://github.com/gakonst/foundry/issues/new) with the question.

Join the [foundry telegram](https://t.me/foundry_rs) to chat with the community!

## Acknowledgements

- Foundry is a clean-room rewrite of the testing framework
  [dapptools](https://github.com/dapphub/dapptools). None of this would have
  been possible without the DappHub team's work over the years.
- [Matthias Seitz](https://twitter.com/mattsse_): Created
  [ethers-solc](https://github.com/gakonst/ethers-rs/tree/master/ethers-solc/)
  which is the backbone of our compilation pipeline, as well as countless
  contributions to ethers, in particular the `abigen` macros.
- [Rohit Narurkar](https://twitter.com/rohitnarurkar): Created the Rust Solidity
  version manager [svm-rs](https://github.com/roynalnaruto/svm-rs) which we use
  to auto-detect and manage multiple Solidity versions.
- All the other
  [contributors](https://github.com/gakonst/foundry/graphs/contributors) to the
  [ethers-rs](https://github.com/gakonst/ethers-rs) &
  [foundry](https://github.com/gakonst/foundry) repositories and chatrooms.
