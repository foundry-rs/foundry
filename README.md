# <h1 align="center"> dapptools.rs </h1>

_Rust port of DappTools_

![Github Actions](https://github.com/gakonst/dapptools-rs/workflows/Tests/badge.svg)
[![Telegram Chat](https://img.shields.io/endpoint?color=neon&style=flat-square&url=https%3A%2F%2Ftg.sumanjay.workers.dev%2Fturbodapptools)](https://t.me/turbodapptools)
[![Crates.io][crates-badge]][crates-url]

[crates-badge]: https://img.shields.io/crates/v/turbodapp.svg
[crates-url]: https://crates.io/crates/turbodapp

## Installing

We have not published a release yet. Until we do, please use the command below.
Because our dependencies may not be stable, do not forget the `--locked`
parameter, which will force the installer to use the checked in `Cargo.lock`
file.

```
cargo install --git https://github.com/gakonst/dapptools-rs --locked
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

## Features

- seth
  - [ ] `--abi-decode`
  - [ ] `--calldata-decode`
  - [x] `--from-ascii` (with `--from-utf8` alias)
  - [ ] `--from-bin`
  - [ ] `--from-fix`
  - [ ] `--from-wei`
  - [ ] `--max-int`
  - [ ] `--max-uint`
  - [ ] `--min-int`
  - [x] `--to-checksum-address` (`--to-address` in dapptools)
  - [x] `--to-ascii`
  - [x] `--to-bytes32`
  - [x] `--to-dec`
  - [x] `--to-fix`
  - [x] `--to-hex`
  - [x] `--to-hexdata`
  - [ ] `--to-int256`
  - [x] `--to-uint256`
  - [x] `--to-wei`
  - [ ] `4byte`
  - [ ] `4byte-decode`
  - [ ] `4byte-event`
  - [ ] `abi-encode`
  - [x] `age`
  - [x] `balance`
  - [x] `basefee`
  - [x] `block`
  - [x] `block-number`
  - [ ] `bundle-source`
  - [x] `call` (partial)
  - [x] `calldata`
  - [x] `chain`
  - [x] `chain-id`
  - [ ] `code`
  - [ ] `debug`
  - [ ] `estimate`
  - [ ] `etherscan-source`
  - [ ] `events`
  - [x] `gas-price`
  - [ ] `index`
  - [x] `keccak`
  - [ ] `logs`
  - [x] `lookup-address`
  - [ ] `ls`
  - [ ] `mktx`
  - [x] `namehash`
  - [ ] `nonce`
  - [ ] `publish`
  - [ ] `receipt`
  - [x] `resolve-name`
  - [ ] `run-tx`
  - [x] `send` (partial)
  - [ ] `sign`
  - [x] `storage`
  - [ ] `tx`
- dapp
  - [ ] test
    - [x] Simple unit tests
      - [x] Gas costs
      - [x] DappTools style test output
      - [x] JSON test output
      - [x] Matching on regex
      - [x] DSTest-style assertions support
    - [x] Fuzzing
    - [ ] Symbolic execution
    - [ ] Coverage
    - [ ] HEVM-style Solidity cheatcodes
      - [x] roll
      - [x] warp
      - [x] ffi
      - [x] store
      - [x] load
      - [ ] sign
      - [ ] addr
      - [ ] makeEOA
      - ...?
    - [ ] Structured tracing with abi decoding
    - [ ] Per-line gas profiling
    - [x] Forking mode
    - [x] Automatic solc selection
  - [x] build
    - [x] Can read DappTools-style .sol.json artifacts
    - [x] Manual remappings
    - [x] Automatic remappings
    - [x] Multiple compiler versions
    - [ ] Incremental compilation
    - [ ] Can read Hardhat-style artifacts
    - [ ] Can read Truffle-style artifacts
  - [x] install
  - [x] update
  - [ ] debug
  - [x] CLI Tracing with `RUST_LOG=dapp=trace`

## Tested Against

This repository has been tested against a few repositories which you can monitor
[here](https://github.com/gakonst/dapptools-benchmarks)

## Development

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
[issue](https://github.com/gakonst/dapptools-rs/issues/new) with the question.

Join the [turbodapptools telegram](https://t.me/turbodapptools) to chat with the
community!
