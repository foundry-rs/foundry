# <h1 align="center"> dapptools.rs </h1>

*Rust port of DappTools*

![Github Actions](https://github.com/gakonst/dapptools-rs/workflows/Tests/badge.svg)


## `dapp` example Usage

### Run Solidity tests

Any contract that contains a function starting with `test` is being tested. The glob
passed to `--contracts` must be wrapped with quotes so that it gets passed to the internal
command without being expanded by your shell.

```bash
$ cargo r --bin dapp test --contracts './**/*.sol'
    Finished dev [unoptimized + debuginfo] target(s) in 0.21s
     Running `target/debug/dapp test --contracts './**/*.sol'`
Running 1 tests for Foo
[PASS] testX (gas: 267)

Running 1 tests for GmTest
[PASS] testGm (gas: 25786)

Running 1 tests for FooBar
[PASS] testX (gas: 267)

Running 3 tests for GreeterTest
[PASS] testIsolation (gas: 3702)
[PASS] testFailGreeting (gas: 26299)
[PASS] testGreeting (gas: 26223)
```

You can optionally specify a regular expresion, to only run matching functions:

```bash
$ cargo r --bin dapp test --contracts './**/*.sol' -m testG
    Finished dev [unoptimized + debuginfo] target(s) in 0.26s
     Running `target/debug/dapp test --contracts './**/*.sol' -m testG`
Running 1 tests for GreeterTest
[PASS] testGreeting (gas: 26223)

Running 1 tests for GmTest
[PASS] testGm (gas: 25786)
```

### Test output as JSON

In order to compose with other commands, you may print the results as JSON via the `--json` flag

```bash
$ ./target/release/dapp test -c "./**/*.sol" --json
{"GreeterTest":{"testIsolation":{"success":true,"gas_used":3702},"testFailGreeting":{"success":true,"gas_used":26299},"testGreeting":{"success":true,"gas_used":26223}},"FooBar":{"testX":{"success":true,"gas_used":267}},"Foo":{"testX":{"success":true,"gas_used":267}},"GmTest":{"testGm":{"success":true,"gas_used":25786}}}
```

### Build the contracts

You can build the contracts by running, which will by default output the compilation artifacts
of all contracts under `src/` at `out/dapp.sol.json`:

```bash
$ ./target/release/dapp build
```

You can specify an alternative path for your contracts and libraries with `--remappings`, `--lib-path`
and `--contracts`. We default to importing libraries from `./lib`, but you still need to manually
set your remappings.

In the example below, we see that this also works for importing libraries from different paths
(e.g. having a DappTools-style import under `lib/` and an NPM-style import under `node_modules`)

Notably, we need 1 remapping and 1 lib path for each import. Given that this can be tedious,
you can do set remappings via the env var `DAPP_REMAPPINGS`, by setting your remapping  1 in each line

```bash
$ dapp build --out out.json \
    --remappings ds-test/=lib/ds-test/src/ \
    --lib-paths ./lib/
    --remappings @openzeppelin/=node_modules/@openzeppelin/ \
    --lib-path ./node_modules/@openzeppelin
```


```bash
$ echo $DAPP_REMAPPINGS
@openzeppelin/=lib/openzeppelin-contracts/
ds-test/=lib/ds-test/src/
$ dapp build --out out.json \
    --lib-paths ./lib/ \
    --lib-paths ./node_modules/@openzeppelin
```

## Development

### Rust Toolchain

We use the stable Rust toolchain. Install by running: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`

### Building & testing

```
cargo check
cargo test
cargo doc --open
cargo build [--release]
```
**Tip**: If you encounter the following error when building the project, please update your Rust toolchain with `rustup update`.

```
error[E0658]: use of unstable library feature 'map_into_keys_values'
```

### CLI Help

The CLI options can be seen below. You can fully customize the initial blockchain
context. As an example, if you pass the flag `--block-number`, then the EVM's `NUMBER`
opcode will always return the supplied value. This can be useful for testing.


#### Build

```bash
$ cargo r --bin dapp build --help
   Compiling dapptools v0.1.0
    Finished dev [unoptimized + debuginfo] target(s) in 3.45s
     Running `target/debug/dapp build --help`
dapp-build 0.1.0
build your smart contracts

USAGE:
    dapp build [FLAGS] [OPTIONS] [--] [remappings-env]

FLAGS:
    -h, --help          Prints help information
    -n, --no-compile    skip re-compilation
    -V, --version       Prints version information

OPTIONS:
    -c, --contracts <contracts>         glob path to your smart contracts [default: ./src/**/*.sol]
        --evm-version <evm-version>     choose the evm version [default: berlin]
        --lib-path <lib-path>           the path where your libraries are installed
    -o, --out <out-path>                path to where the contract artifacts are stored [default: ./out/dapp.sol.json]
    -r, --remappings <remappings>...    the remappings

ARGS:
    <remappings-env>     [env: DAPP_REMAPPINGS=]
```

#### Test

```bash
$ cargo r --bin dapp test --help
    Finished dev [unoptimized + debuginfo] target(s) in 0.31s
     Running `target/debug/dapp test --help`
dapp-test 0.1.0
build your smart contracts

USAGE:
    dapp test [FLAGS] [OPTIONS] [--] [remappings-env]

FLAGS:
    -h, --help          Prints help information
    -j, --json          print the test results in json format
    -n, --no-compile    skip re-compilation
    -V, --version       Prints version information

OPTIONS:
        --block-coinbase <block-coinbase>
            the block.coinbase value during EVM execution [default: 0x0000000000000000000000000000000000000000]

        --block-difficulty <block-difficulty>    the block.difficulty value during EVM execution [default: 0]
        --block-gas-limit <block-gas-limit>      the block.gaslimit value during EVM execution
        --block-number <block-number>            the block.number value during EVM execution [default: 0]
        --block-timestamp <block-timestamp>      the block.timestamp value during EVM execution [default: 0]
        --chain-id <chain-id>                    the chainid opcode value [default: 1]
    -c, --contracts <contracts>                  glob path to your smart contracts [default: ./src/**/*.sol]
        --evm-version <evm-version>              choose the evm version [default: berlin]
        --gas-limit <gas-limit>                  the block gas limit [default: 25000000]
        --gas-price <gas-price>                  the tx.gasprice value during EVM execution [default: 0]
        --lib-path <lib-path>                    the path where your libraries are installed
    -o, --out <out-path>
            path to where the contract artifacts are stored [default: ./out/dapp.sol.json]

    -m, --match <pattern>                        only run test methods matching regex [default: .*]
    -r, --remappings <remappings>...             the remappings
        --tx-origin <tx-origin>
            the tx.origin value during EVM execution [default: 0x0000000000000000000000000000000000000000]


ARGS:
    <remappings-env>     [env: DAPP_REMAPPINGS=]

```

## Features

* seth
    * [x] `--from-ascii`
    * [x] `--to-checksum-address`
    * [x] `--to-bytes32`
    * [x] `block`
    * [x] `call` (partial)
    * [x] `send` (partial)
    * [x] `balance`
    * [x] `ens`
* dapp
    * [ ] test
        * [x] simple unit tests
            * [x] Gas costs
            * [x] DappTools style test output
            * [x] JSON test output
            * [x] matching on regex
        * [ ] fuzzing
        * [ ] symbolic execution
        * [ ] coverage
        * [ ] HEVM-style Solidity cheatcodes
        * [ ] structured tracing with abi decoding
        * [ ] per-line gas profiling
        * [ ] forking mode
        * [x] automatic solc selection
    * [x] build
        * [x] can read DappTools-style .sol.json artifacts
        * [x] remappings
        * [x] multiple compiler versions
        * [ ] incremental compilation
        * [ ] can read Hardhat-style artifacts
        * [ ] can read Truffle-style artifacts
    * [ ] debug
