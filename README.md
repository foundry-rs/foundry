# <h1 align="center"> dapptools.rs </h1>

*Rust port of DappTools*

![Github Actions](https://github.com/gakonst/dapptools-rs/workflows/Tests/badge.svg)

## Supported Commands

* seth
    * [x] `--from-ascii`
    * [x] `--to-checksum-address`
    * [ ] `--from-bin`: isn't this the same as from-ascii?
* dapp
    * [ ]
* hevm
    * TBD / May be replaced with [rust-cevm](https://github.com/brockelmore/rust-cevm/) or [evmc](https://github.com/ethereum/evmc/blob/master/examples/example-rust-vm/src/lib.rs#L12)
