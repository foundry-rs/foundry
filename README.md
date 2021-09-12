# <h1 align="center"> dapptools.rs </h1>

*Rust port of DappTools*

![Github Actions](https://github.com/gakonst/dapptools-rs/workflows/Tests/badge.svg)

## Supported Commands

* seth
    * [x] `--from-ascii`
    * [x] `--to-checksum-address`
    * [x] `--to-bytes32`
    * [x] `block`
    * [x] `call` (partial)
    * [x] `send` (partial)
* dapp
    * [ ] test
        * [x] simple unit tests
            * [x] Gas costs
            * [x] DappTools style test output
            * [x] JSON test output
        * [ ] fuzzing
        * [ ] symbolic execution
        * [ ] coverage
    * [ ] build
        * [x] can read DappTools-style .sol.json artifacts
        * [ ] remappings
    * [ ] debug
