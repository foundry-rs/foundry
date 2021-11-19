# <h1 align="center"> foundry </h1>

**Foundry is a blazing fast, portable and modular toolkit for Ethereum
application development written in Rust.**

![Github Actions](https://github.com/gakonst/foundry/workflows/Tests/badge.svg)
[![Telegram Chat](https://img.shields.io/endpoint?color=neon&style=flat-square&url=https%3A%2F%2Ftg.sumanjay.workers.dev%2Ffoundry_rs)](https://t.me/foundry_rs)
[![Crates.io][crates-badge]][crates-url]

[crates-badge]: https://img.shields.io/crates/v/foundry.svg
[crates-url]: https://crates.io/crates/foundry

## Directory structure

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
- [`ark-serialize`](serialize): Provides efficient serialization and point
  compression for finite fields and elliptic curves

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

## Why?! DappTools is great!

Developer experience is the #1 thing we should be optimizing for in development.
Tests MUST be fast, non-trivial tests (e.g. proptests) MUST be easy to write,
and compilation MUST be fast.

Before getting into technical reasons, my simple answer is: rewriting software
in Rust is fun. I enjoy it, and that could be the end of the "why" section.

DappTools is REALLY great.
[You should try it](https://github.com/dapphub/dapptools/), especially the
symbolic execution and step debugger features.

But it has some shortcomings:

It's written in a mix of Bash, Javascript and Haskell. In my opinion, this makes
it hard to contribute, you don't have a "standard" way to test things, and it
happens to be that there are not that many Haskell developers in the Ethereum
community.

It is also hard to distribute. It requires installing Nix, and that's a barrier
to entry to many already because (for whatever reason) Nix doesn't always
install properly the first time.

The more technical reasons I decided to use it are:

1. It is easier to write regression tests in Rust than in Bash.
1. Rust binaries are cross-platform and easy to distribute.
1. Compilation speed: We can use native bindings to the Solidity compiler
   (instead of calling out to solcjs or even to the compiled binary) for extra
   compilation speed.
1. Testing speed: HEVM tests are really fast, but I believe we can go faster by
   leveraging Rust's high performance multithreading and resource allocation
   system.
1. There seems to be an emerging community of Rust-Ethereum developers.

Benchmarks TBD in the future, but:

1. [Using a Rust EVM w/ forked RPC mode](https://github.com/brockelmore/rust-cevm/#compevm-rust-ethereum-virtual-machine-implementation-designed-for-smart-contract-composability-testing)
   was claimed to be as high as 10x faster than HEVM's forking mode.
1. Native bindings to the Solidity compiler have been shown to be
   [10x](https://forum.openzeppelin.com/t/a-faster-solidity-compiler-cli-in-rust/2546)
   faster than the JS bindings or even just calling out to the native binary.
1. `seth` and `dapp` are less than 7mb when built with `cargo build --release`.

## Is this actually working?

This repository has been tested against a few DappTools repositories which you
can monitor [here](https://github.com/gakonst/dapptools-benchmarks).

## Development

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
  [dapptools](https://github.com/dapphub/dapptools).
- Matthias Seitz: Ethers-solc, abigen
- Rohit Narunkar: SVM
- ...
