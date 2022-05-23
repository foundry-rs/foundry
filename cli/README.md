# Foundry CLIs

The CLIs are written using [clap's](https://docs.rs/clap) [derive feature](https://github.com/clap-rs/clap/blob/master/examples/derive_ref/README.md).

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
    bind               Generate rust bindings for your smart contracts
    build              Build your smart contracts
    cache              Manage the foundry cache
    clean              Removes the build artifacts and cache directories
    completions        Generate shell completions script
    config             Shows the currently set config values
    create             Deploy a compiled contract
    flatten            Concats a file with all of its imports
    help               Print this message or the help of the given subcommand(s)
    init               Initializes a new forge sample project
    install            Installs one or more dependencies as git submodules
    remappings         Prints the automatically inferred remappings for this repository
    remove             Removes one or more dependencies from git submodules
    run                Run a single smart contract as a script
    snapshot           Creates a snapshot of each test's gas usage
    test               Test your smart contracts
    update             Fetches all upstream lib changes
    verify-check       Check verification status on Etherscan. Requires `ETHERSCAN_API_KEY` to be set.
    verify-contract    Verify your smart contracts source code on Etherscan. Requires `ETHERSCAN_API_KEY` to be set.
```

The subcommands are also aliased to their first letter, e.g. you can do
`forge t` instead of `forge test` or `forge b` instead of `forge build`.

### Build

The `build` subcommand proceeds to compile your smart contracts.

```
forge-build
build your smart contracts

USAGE:
    forge build [OPTIONS]

OPTIONS:
    -c, --contracts <CONTRACTS>
            the directory relative to the root under which the smart contracts are [env: DAPP_SRC=]
        --evm-version <EVM_VERSION>
            choose the evm version [default: london]
        --force
            force recompilation of the project, deletes the cache and artifacts folders
    -h, --help
            Print help information
        --hardhat
            uses hardhat style project layout. This a convenience flag and is the same as `--contracts contracts --lib-
            paths node_modules`
        --ignored-error-codes <IGNORED_ERROR_CODES>
            ignore warnings with specific error codes
        --lib-paths <LIB_PATHS>
            the paths where your libraries are installed
        --libraries <LIBRARIES>
            add linked libraries
        --no-auto-detect
            if set to true, skips auto-detecting solc and uses what is in the user's $PATH
    -o, --out <OUT_PATH>
            path to where the contract artifacts are stored
        --optimize
            activate the solidity optimizer
        --optimize-runs <OPTIMIZE_RUNS>
            optimizer parameter runs [default: 200]
    -r, --remappings <REMAPPINGS>
            the remappings
        --remappings-env <REMAPPINGS_ENV>
            [env: DAPP_REMAPPINGS=]
        --root <ROOT>
            the project's root path. By default, this is the root directory of the current Git repository or the current
            working directory if it is not part of a Git repository
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

#### Remappings

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
forge-test
test your smart contracts

USAGE:
    forge test [OPTIONS]

OPTIONS:
    -j, --json
            print the test results in json format
        --gas-limit <GAS_LIMIT>
            the block gas limit [default: 18446744073709551615]
        --chain-id <CHAIN_ID>
            the chainid opcode value [default: 1]
        --gas-price <GAS_PRICE>
            the tx.gasprice value during EVM execution [default: 0]
        --block-base-fee-per-gas <BLOCK_BASE_FEE_PER_GAS>
            the base fee in a block [default: 0]
        --tx-origin <TX_ORIGIN>
            the tx.origin value during EVM execution [default: 0x0000000000000000000000000000000000000000]
        --block-coinbase <BLOCK_COINBASE>
            the block.coinbase value during EVM execution [default: 0x0000000000000000000000000000000000000000]
        --block-timestamp <BLOCK_TIMESTAMP>
            the block.timestamp value during EVM execution [env: DAPP_TEST_TIMESTAMP=] [default: 0]
        --block-number <BLOCK_NUMBER>
            the block.number value during EVM execution [env: DAPP_TEST_NUMBER=] [default: 0]
        --block-difficulty <BLOCK_DIFFICULTY>
            the block.difficulty value during EVM execution [default: 0]
        --block-gas-limit <BLOCK_GAS_LIMIT>
            the block.gaslimit value during EVM execution
    -e, --evm-type <EVM_TYPE>
            the EVM type you want to use (e.g. sputnik) [default: sputnik]
    -f, --fork-url <FORK_URL>
            fetch state over a remote instead of starting from empty state
        --fork-block-number <FORK_BLOCK_NUMBER>
            pins the block number for the state fork [env: DAPP_FORK_BLOCK=]
        --initial-balance <INITIAL_BALANCE>
            the initial balance of each deployed test contract [default: 0xffffffffffffffffffffffff]
        --sender <SENDER>
            the address which will be executing all tests [env: DAPP_TEST_ADDRESS=] [default:
            0x0000000000000000000000000000000000000000]
        --ffi
            enables the FFI cheatcode
    -v, --verbosity
            Verbosity mode of EVM output as number of occurrences of the `v` flag (-v, -vv, -vvv, etc.)
                3: print test trace for failing tests
                4: always print test trace, print setup for failing tests
                5: always print test trace and setup
        --debug
            enable debugger
    -m, --match <PATTERN>
            only run test methods matching regex (deprecated, see --match-test, --match-contract)
        --match-test <TEST_PATTERN>
            only run test methods matching regex
        --no-match-test <TEST_PATTERN_INVERSE>
            only run test methods not matching regex
        --match-contract <CONTRACT_PATTERN>
            only run test methods in contracts matching regex
        --no-match-contract <CONTRACT_PATTERN_INVERSE>
            only run test methods in contracts not matching regex
        --root <ROOT>
            the project's root path. By default, this is the root directory of the current Git repository or the current
            working directory if it is not part of a Git repository
    -c, --contracts <CONTRACTS>
            the directory relative to the root under which the smart contracts are [env: DAPP_SRC=]
    -r, --remappings <REMAPPINGS>
            the remappings
        --remappings-env <REMAPPINGS_ENV>
            [env: DAPP_REMAPPINGS=]
        --lib-paths <LIB_PATHS>
            the paths where your libraries are installed
    -o, --out <OUT_PATH>
            path to where the contract artifacts are stored
        --evm-version <EVM_VERSION>
            choose the evm version [default: london]
        --optimize
            activate the solidity optimizer
        --optimize-runs <OPTIMIZE_RUNS>
            optimizer parameter runs [default: 200]
        --ignored-error-codes <IGNORED_ERROR_CODES>
            ignore warnings with specific error codes
        --no-auto-detect
            if set to true, skips auto-detecting solc and uses what is in the user's $PATH
        --force
            force recompilation of the project, deletes the cache and artifacts folders
        --hardhat
            uses hardhat style project layout. This a convenience flag and is the same as `--contracts contracts --lib-
            paths node_modules`
        --libraries <LIBRARIES>
            add linked libraries
        --allow-failure
            if set to true, the process will exit with an exit code = 0, even if the tests fail [env:
            FORGE_ALLOW_FAILURE=]
    -h, --help
            Print help information
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
no files changed, compilation skipped
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
no files changed, compilation skipped
{"\"Gm.json\":Gm":{"testNonOwnerCannotGm":{"success":true,"reason":null,"gas_used":3782,"counterexample":null,"logs":[]},"testOwnerCannotGmOnBadBlocks":{"success":true,"reason":null,"gas_used":7771,"counterexample":null,"logs":[]},"testOwnerCanGmOnGoodBlocks":{"success":true,"reason":null,"gas_used":31696,"counterexample":null,"logs":[]}},"\"Greet.json\":Greet":{"testWorksForAllGreetings":{"success":true,"reason":null,"gas_used":null,"counterexample":null,"logs":[]},"testCannotGm":{"success":true,"reason":null,"gas_used":6819,"counterexample":null,"logs":[]},"testCanSetGreeting":{"success":true,"reason":null,"gas_used":31070,"counterexample":null,"logs":[]}}}
```

#### Running a Subset of Tests

By default, `forge test` (and `forge snapshot`) will run every function in any contract if the function starts with `test`.

You can narrow down the amount of tests to run by using one or more of the current available command line arguments for matching tests.

```
--match-test <TEST_PATTERN>
--no-match-test <TEST_PATTERN_INVERSE>
--match-contract <CONTRACT_PATTERN>
--no-match-contract <CONTRACT_PATTERN_INVERSE>
```

#### Examples

`--match-contract` and `--no-match-contract` matches against the name of the contracts containing tests. Consider the following contracts, each containing a few tests.

```solidity
contract ContractFoo { ... }
contract ContractFooBar { ... }
contract ContractBar { ... }
```

* `forge test --match-contract Contract` will run the tests in all of those contracts
* `forge test --match-contract Foo` will run the tests in the `ContractFoo` and `ContractFooBar`
* `forge test --match-contract "Foo$"` will only run the tests in `ContractFoo`
* `forge test --match-contract "ContractFoo|ContractBar"` will only run the tests of contracts including `ContractFoo` or `ContractBar` in it's name
* `forge test --no-match-contract FooBar` will run the tests in `ContractFoo` and `ContractBar`


`--match-test` and `--no-match-test` matches against the test function names, by default they start with the `test` prefix. Consider the following contracts with a few test functions.

```solidity
contract ContractFoo {
  func testFoo() {}
  func testFooBar() {}
}

contract ContractBar {
  func testBar() {}
  func testFooBar() {}
}
```

* `forge test --match-test testFoo` will run all tests except for `ContractBar.testBar`
* `forge test --match-test "Foo$"` will run only `ContractFoo.testFoo`
* `forge test --match-test "testFoo$|testBar$"` will run `ContractFoo.testFoo` and `ContractBar.testBar`
* `forge test --no-match-test Bar` will only run `ContractBar.testBar`
* `forge test --no-match-test test` will run no tests

You can always combine any of the four arguments, they have AND semantics.

### Inspect

The `inspect` subcommand compiles the specified contract and prints the specified mode.

To run the `inspect` command, run `forge inspect <CONTRACT> <MODE>`.
Where `<CONTRACT>` is the name of the contract to inspect, and `<MODE>` is the compiled artifact output field.

`<MODE>` can be one of the following:
- `abi`
- `bytecode`
- `deployed-bytecode`
- `asm`
- `asm-optimized`
- `method-identifiers`
- `gas-estimates`
- `storage-layout`
- `dev-doc`
- `user-doc`
- `ir`
- `ir-optimiized`
- `metadata`
- `ewasm`

For example, to get the bytecode of `Greeter.sol` in this project structure:
```ml
src
└─ Greeter.sol
```
run: `forge inspect Greeter bytecode`

Which will output the contract bytecode as a hex string.

TIP: To save this easily to a file (for example `output.txt`),
you can redirect the output of `forge inspect` to the file like so:
`forge inspect Greeter bytecode > output.txt`

##### Forge Inspect Command Docs

Output of `forge inspect --help`:
```
forge-inspect
Outputs a contract in a specified format (ir, assembly, ...)

USAGE:
    forge inspect [OPTIONS] <CONTRACT> <MODE>

ARGS:
    <CONTRACT>
            the contract to inspect

    <MODE>
            the contract artifact field to inspect
```

### Common Patterns

A few common patterns to help with your development workflow.

#### Tracing

When you're trying to find out why a specific test is failing, you can clean up the logs a bit by running only the test you're tracing:

```console
forge test --match-contract MyContractTest --match-test "testBar$" -vvv
# use $ to indicate end of line, otherwise it can match testBarFoo, testBarFooBar etc.
# `--match-contract` is only necessary if you have multiple tests with the same name
```

#### Separating Tests

You might want to run your different kind of tests separately, for example, unit tests vs benchmark, you can suffix the contract name with the type of test to run them separately.

```solidity
contract FooUnitTest {}
contract BarUnitTest {}
contract FooBenchmark {}
contract BarBenchmark {}
```

* To run unit tests `forge test --match-contract "UnitTest$"`
* To get a gas snapshot only of the benchmark tests `forge snapshot --match-contract "Benchmark$"`

### Edge cases

If you have two tests with the same name but different arity (number of arguments), you can't run them individually.

```solidity
function testFoo() public { assert(1 == 1); }
function testFoo(uint256 bar) public { assert(bar == bar); }
```

## cast

```
foundry-cli
Perform Ethereum RPC calls from the comfort of your command line.

USAGE:
    cast <SUBCOMMAND>

OPTIONS:
    -h, --help    Print help information

SUBCOMMANDS:
    --abi-decode             Decode ABI-encoded hex output data. Pass --input to decode as input, or use
                             `--calldata-decode`
    --calldata-decode        Decode ABI-encoded hex input data. Use `--abi-decode` to decode output data
    --from-utf8              convert text data into hexdata
    --from-fix               convert fixed point into specified number of decimals
    --from-wei               convert wei into an ETH amount
    --max-int                maximum i256 value
    --max-uint               maximum u256 value
    --min-int                minimum i256 value
    --to-ascii               convert hex data to text data
    --to-bytes32             right-pads a hex bytes string to 32 bytes
    --to-checksum-address    convert an address to a checksummed format (EIP-55)
    --to-dec                 convert hex value into decimal number
    --to-fix                 convert integers into fixed point with specified decimals
    --to-hex                 convert a decimal number into hex
    --to-hexdata             [<hex>|</path>|<@tag>]
                                 Output lowercase, 0x-prefixed hex, converting from the
                                 input, which can be:
                                   - mixed case hex with or without 0x prefix
                                   - 0x prefixed hex, concatenated with a ':'
                                   - absolute path to file
                                   - @tag, where $TAG is defined in environment variables
    --to-uint256             convert a number into uint256 hex string with 0x prefix
    --to-int256              convert a number into int256 hex string with 0x prefix
    --to-wei                 convert an ETH amount into wei
    4byte                    Fetches function signatures given the selector from 4byte.directory
    4byte-decode             Decodes transaction calldata by fetching the signature using 4byte.directory
    4byte-event              Takes a 32 byte topic and prints the response from querying 4byte.directory for that topic
    pretty-calldata          Pretty prints calldata, if available gets signature from 4byte.directory
    abi-encode
    age                      Prints the timestamp of a block
    balance                  Print the balance of <account> in wei
    basefee                  Print the basefee of a block
    block                    Prints information about <block>. If <field> is given, print only the value of that
                             field
    block-number             Prints latest block number
    call                     Perform a local call to <to> without publishing a transaction.
    calldata                 Pack a signature and an argument list into hexadecimal calldata.
    chain                    Prints symbolic name of current blockchain by checking genesis hash
    chain-id                 returns ethereum chain id
    code                     Prints the bytecode at <address>
    completions              generate shell completions script
    estimate                 Estimate the gas cost of a transaction from <from> to <to> with <data>
    gas-price                Prints current gas price of target chain
    index                    Get storage slot of value from mapping type, mapping slot number and input value
    help                     Print this message or the help of the given subcommand(s)
    keccak                   Keccak-256 hashes arbitrary data
    lookup-address           Returns the name the provided address resolves to
    namehash                 returns ENS namehash of provided name
    nonce                    Prints the number of transactions sent from <address>
    resolve-name             Returns the address the provided ENS name resolves to
    send                     Publish a transaction signed by <from> to call <to> with <data>
    storage                  Show the raw value of a contract's storage slot
    tx                       Show information about the transaction <tx-hash>
    wallet                   Set of wallet management utilities
```
