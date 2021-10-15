# <h1 align="center"> dapptools.rs </h1>

*Rust port of DappTools*

![Github Actions](https://github.com/gakonst/dapptools-rs/workflows/Tests/badge.svg)

## Why?! DappTools is great!

Developer experience is the #1 thing we should be optimizing for in development. Tests MUST be fast, non-trivial tests (e.g. proptests) 
MUST be easy to write, compilation MUST be fast.

Before getting into technical reasons, my simple answer is: rewriting software in Rust is fun. I enjoy it, and that could be the end of the "why" section.

DappTools is REALLY great. [You should try it](https://github.com/dapphub/dapptools/), especially the symbolic execution
and step debugger features.

But it has some shortcomings:

It's written in a mix of Bash, Javascript and Haskell. In my opinion, this makes it 
hard to contribute, you don't have a "standard" way to test things, and it happens to be
that there are not that many Haskell developers in the Ethereum community.

It is also hard to distribute. It requires installing Nix, and that's a barrier to entry
to many already because (for whatever reason) Nix doesn't always install properly the first time.

The more technical reasons I decided to use it are:
1. It is easier to write regression tests in Rust vs in Bash
1. Rust binaries are cross-platform and easy to distribute
1. Compilation speed: We can use native bindings to the Solidity compiler (instead of calling out to solcjs or even to the compiled binary) for extra compilation speed
1. Testing speed: HEVM tests are really fast, but I believe we can go faster by leveraging Rust's high performance multithreading and resource allocation system.
1. There seems to be an emerging community of Rust-Ethereum developers

Benchmarks TBD in the future, but:
1. [Using a Rust EVM w/ forked RPC mode](https://github.com/brockelmore/rust-cevm/#compevm-rust-ethereum-virtual-machine-implementation-designed-for-smart-contract-composability-testing) was claimed to be as high as 10x faster than HEVM's forking mode.
1. Native bindings to the Solidity compiler have shown to be [10x](https://forum.openzeppelin.com/t/a-faster-solidity-compiler-cli-in-rust/2546) faster than the JS bindings or even just calling out to the native binary
 1. `seth` and `dapp` are less than 7mb when built with `cargo build --release`

## Features

* seth
    * [x] `--from-ascii` / `--from-utf8`
    * [x] `--to-hex`
    * [x] `--to-checksum-address`
    * [x] `--to-ascii`
    * [x] `--to-bytes32`
    * [x] `--to-dec`
    * [x] `--to-fix`
    * [x] `--to-uint256`
    * [x] `--to-wei`
    * [x] `block`
    * [x] `block-number`
    * [x] `chain-id`
    * [x] `age`
    * [x] `basefee`
    * [x] `gas-price`
    * [x] `call` (partial)
    * [x] `send` (partial)
    * [x] `balance`
    * [x] `ens`
    * abi encoding
    * 4byte resolution
* dapp
    * [ ] test
        * [x] simple unit tests
            * [x] Gas costs
            * [x] DappTools style test output
            * [x] JSON test output
            * [x] matching on regex
            * [x] DSTest-style assertions support
        * [x] fuzzing
        * [ ] symbolic execution
        * [ ] coverage
        * [ ] HEVM-style Solidity cheatcodes
            * [x] roll
            * [x] warp
            * [x] ffi
            * [x] store
            * [x] load
            * [ ] sign
            * [ ] addr
            * [ ] makeEOA
            * ...?
        * [ ] structured tracing with abi decoding
        * [ ] per-line gas profiling
        * [x] forking mode
        * [x] automatic solc selection
    * [x] build
        * [x] can read DappTools-style .sol.json artifacts
        * [x] manual remappings
        * [x] automatic remappings
        * [x] multiple compiler versions
        * [ ] incremental compilation
        * [ ] can read Hardhat-style artifacts
        * [ ] can read Truffle-style artifacts
    * [x] install
    * [x] update
    * [ ] debug
    * [x] CLI Tracing with `RUST_LOG=dapp=trace`

## Tested Against

This repository has been tested against the following DappTools repos:
* [LootLoose](https://github.com/gakonst/lootloose/) (minus the FFI tests)
* [solmate](https://github.com/Rari-Capital/solmate/) (replace all the `prove` symbolic tests with fuzz tests, and skip `invariant` tests for now)

## Development

### Rust Toolchain

We use the stable Rust toolchain. Install by running: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`

#### Minimum Supported Rust Version

The current minimum supported Rust version is `rustc 1.54.0 (a178d0322 2021-07-26)`.

### Building & testing

```
cargo check
cargo test
cargo doc --open
cargo build [--release]
```

### Formatting

```
cargo +nightly fmt
cargo clippy
```
## Getting Help

First, see if the answer to your question can be found in the API documentation. If the answer
is not there, try opening an [issue](https://github.com/gakonst/dapptools-rs/issues/new) with the question.

Join the [turbodapptools telegram](https://t.me/turbodapptools) to chat with the community!
