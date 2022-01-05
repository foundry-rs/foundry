# Foundry CLIs

The CLIs are written using [structopt](https://docs.rs/structopt).

Debug logs are printed with
[`tracing`](https://docs.rs/tracing/0.1.29/tracing/). You can configure the
verbosity level via the
[`RUST_LOG`](https://docs.rs/tracing-subscriber/0.3.2/tracing_subscriber/fmt/index.html#filtering-events-with-environment-variables)
environment variable, on a per package level,
e.g.:`RUST_LOG=forge=trace,evm_adapters=trace forge test`

## Forge

```
foundry-cli 0.1.0
Build, test, fuzz, formally verify, debug & deploy solidity contracts.

USAGE:
    forge <SUBCOMMAND>

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

SUBCOMMANDS:
    build              build your smart contracts
    clean              removes the build artifacts and cache directories completions
    create             deploy a compiled contract
    help               Prints this message or the help of the given subcommand(s)
    init               initializes a new forge sample repository
    install            installs one or more dependencies as git submodules
    remappings         prints the automatically inferred remappings for this repository
    test               test your smart contracts
    update             fetches all upstream lib changes
    verify-contract    build your smart contracts. Requires `ETHERSCAN_API_KEY` to be set.
```

The subcommands are also aliased to their first letter, e.g. you can do
`forge t` instead of `forge test` or `forge b` instead of `forge build`.

### Build

The `build` subcommand proceeds to compile your smart contracts.

```
forge-build 0.1.0
build your smart contracts

USAGE:
    forge build [FLAGS] [OPTIONS]

FLAGS:
        --force             force recompilation of the project, deletes the cache and artifacts folders
    -h, --help              Prints help information
        --no-auto-detect    if set to true, skips auto-detecting solc and uses what is in the user's $PATH
    -V, --version           Prints version information

OPTIONS:
    -c, --contracts <contracts>              the directory relative to the root under which the smart contrats are [env:
                                             DAPP_SRC=]
        --evm-version <evm-version>          choose the evm version [default: london]
        --lib-paths <lib-paths>...           the paths where your libraries are installed
    -o, --out <out-path>                     path to where the contract artifacts are stored
    -r, --remappings <remappings>...         the remappings
        --remappings-env <remappings-env>     [env: DAPP_REMAPPINGS=]
        --root <root>                        the project's root path, default being the current working directory
```

By default, it will auto-detect the solc pragma version requirement per-file and
will use the [latest version](https://github.com/ethereum/solidity/releases)
that satisfies the requirement, (e.g. `pragma solidity >=0.7.0 <0.8.0` will use
`solc` 0.7.6). If you want to disable this feature, you can call
`forge build --no-auto-detect`, and it'll use whichever `solc` version is in
your `$PATH`.

The project's root directory defaults to the current working directory, assuming
contracts are under `src/` and `lib/`, but can also be configured via the
`--root`, `--lib-paths` and `--contracts` arguments. The contracts and libraries
directories are assumed to be relative to the project root, for example
`forge build --root ../my-project --contracts my-contracts-dir` will try to find
the contracts under `../my-project/my-contracts-dir`. You can also configure the
output directory where the contract artifacts will be written to with the
`--out` variable.

Compiler remappings are automatically detected, but if you want to override them
you can do it with the `--remappings` flag like below:

```bash
$ forge build --remappings @openzeppelin/=node_modules/@openzeppelin/
```

Alternatively you could provide a `remappings.txt` file in the project root
containing a line separated list of your package names and the path to them. For
example:

```bash
$ cat remappings.txt
ds-test=lib/ds-test/src
@openzeppelin=node_modules/openzeppelin-contracts/contracts
```

This then allows you to import the dependencies in your Solidity code like:

```solidity
import "ds-test/...";
import "@openzeppelin/...";
```

Most of the arguments can also be provided via environment variables, which you
can find by looking for the `env` tooltip in the command's help menu
(`forge build --help`).

### Test

Proceeds to build (if needed) and test your smart contracts. It will look for
any contract with a function name that starts with `test`, deploy it, and run
that test function. If that test function takes any arguments, it will proceed
to "fuzz" it (i.e. call it with a lot of different arguments, default: 256
tries).

The command re-uses all the options of `forge build`, and also allows you to
configure any blockchain context related variables such as the block coinbase,
difficulty etc.

```
forge-test 0.1.0
test your smart contracts

USAGE:
    forge test [FLAGS] [OPTIONS]

FLAGS:
        --ffi               enables the FFI cheatcode
        --force             force recompilation of the project, deletes the cache and artifacts folders
    -h, --help              Prints help information
    -j, --json              print the test results in json format
        --no-auto-detect    if set to true, skips auto-detecting solc and uses what is in the user's $PATH
    -V, --version           Prints version information

OPTIONS:
        --allow-failure <allow-failure>
            if set to true, the process will exit with an exit code = 0, even if the tests fail [env:
            FORGE_ALLOW_FAILURE=]
        --block-base-fee-per-gas <block-base-fee-per-gas>    the base fee in a block [default: 0]
        --block-coinbase <block-coinbase>
            the block.coinbase value during EVM execution [default: 0x0000000000000000000000000000000000000000]

        --block-difficulty <block-difficulty>
            the block.difficulty value during EVM execution [default: 0]

        --block-gas-limit <block-gas-limit>                  the block.gaslimit value during EVM execution
        --block-number <block-number>
            the block.number value during EVM execution [env: DAPP_TEST_NUMBER=]  [default: 0]

        --block-timestamp <block-timestamp>
            the block.timestamp value during EVM execution [env: DAPP_TEST_TIMESTAMP=]  [default: 0]

        --chain-id <chain-id>                                the chainid opcode value [default: 1]
    -c, --contracts <contracts>
            the directory relative to the root under which the smart contrats are [env: DAPP_SRC=]

    -e, --evm-type <evm-type>
            the EVM type you want to use (e.g. sputnik, evmodin) [default: sputnik]

        --evm-version <evm-version>                          choose the evm version [default: london]
        --fork-block-number <fork-block-number>
            pins the block number for the state fork [env: DAPP_FORK_BLOCK=]

    -f, --fork-url <fork-url>
            fetch state over a remote instead of starting from empty state [env: ETH_RPC_URL=]

        --gas-limit <gas-limit>                              the block gas limit [default: 18446744073709551615]
        --gas-price <gas-price>                              the tx.gasprice value during EVM execution [default: 0]
        --initial-balance <initial-balance>
            the initial balance of each deployed test contract [default: 0xffffffffffffffffffffffff]

        --lib-paths <lib-paths>...                           the paths where your libraries are installed
    -o, --out <out-path>                                     path to where the contract artifacts are stored
    -m, --match <pattern>                                    only run test methods matching regex [default: .*]
    -r, --remappings <remappings>...                         the remappings
        --remappings-env <remappings-env>                     [env: DAPP_REMAPPINGS=]
        --root <root>
            the project's root path, default being the current working directory

        --sender <sender>
            the address which will be executing all tests [env: DAPP_TEST_ADDRESS=]  [default:
            0x0000000000000000000000000000000000000000]
        --tx-origin <tx-origin>
            the tx.origin value during EVM execution [default: 0x0000000000000000000000000000000000000000]

        --verbosity <verbosity>                              verbosity of 'forge test' output (0-3) [default: 0]
```

Here's how the CLI output looks like when used with
[`dapptools-template`](https://github.com/gakonst/dapptools-template)

```bash
$ forge test
success.
Running 3 tests for "Greet.json":Greet
[PASS] testCanSetGreeting (gas: 31070)
[PASS] testWorksForAllGreetings (gas: [fuzztest])
[PASS] testCannotGm (gas: 6819)

Running 3 tests for "Gm.json":Gm
[PASS] testOwnerCannotGmOnBadBlocks (gas: 7771)
[PASS] testNonOwnerCannotGm (gas: 3782)
[PASS] testOwnerCanGmOnGoodBlocks (gas: 31696)
```

You can optionally specify a regular expression, to only run matching functions:

```bash
$ forge test -m Cannot
$HOME/oss/foundry/target/release/forge test -m Cannot
no files changed, compilation skippped.
Running 1 test for "Greet.json":Greet
[PASS] testCannotGm (gas: 6819)

Running 2 tests for "Gm.json":Gm
[PASS] testNonOwnerCannotGm (gas: 3782)
[PASS] testOwnerCannotGmOnBadBlocks (gas: 7771)
```

In order to compose with other commands, you may print the results as JSON via
the `--json` flag

```bash
$ forge test --json
no files changed, compilation skippped.
{"\"Gm.json\":Gm":{"testNonOwnerCannotGm":{"success":true,"reason":null,"gas_used":3782,"counterexample":null,"logs":[]},"testOwnerCannotGmOnBadBlocks":{"success":true,"reason":null,"gas_used":7771,"counterexample":null,"logs":[]},"testOwnerCanGmOnGoodBlocks":{"success":true,"reason":null,"gas_used":31696,"counterexample":null,"logs":[]}},"\"Greet.json\":Greet":{"testWorksForAllGreetings":{"success":true,"reason":null,"gas_used":null,"counterexample":null,"logs":[]},"testCannotGm":{"success":true,"reason":null,"gas_used":6819,"counterexample":null,"logs":[]},"testCanSetGreeting":{"success":true,"reason":null,"gas_used":31070,"counterexample":null,"logs":[]}}}
```

```

```
