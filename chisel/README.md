# `chisel`

Chisel is a fast, utilitarian, and verbose solidity REPL. It is heavily inspired by the incredible work done in [soli](https://github.com/jpopesculian/soli) and [solidity-shell](https://github.com/tintinweb/solidity-shell)!

![preview](./assets/preview.gif)

## Checklist

- [ ] REPL functionality
  - [x] Create temporary REPL contract (in memory, or temp file?).
    - [x] Implement `forge-std/Test.sol` so that cheatcodes, etc. can be used.
  - [x] Utilize foundry's `evm` module for REPL env.
    - [x] Implement network forking.
  - [x] Expression evaluation / inspection (i.e. the input `0x01 << 0x08` should inspect a `uint` of value `256`)
    - [x] Support for primitive type expressions (i.e. primitive types, arithmetic ops, bitwise ops, boolean ops, global vars (`msg`, `tx`, `block`, & `abi`))
    - [x] Support for function call expressions (both local and external to the REPL contract)
    - [x] Support for array indexing (external + local)
    - [x] Support for mapping indexing (external + local)
    - [ ] Clean up refactor
  - [x] Input history.
  - [x] Use forge fmt module to format source code when printing via the `!source` command or exporting to a Script file (?)
- [x] Cache REPL History
  - [x] Allow a user to save/load sessions from their Chisel history.
    - [x] Fix session loading bug wrt non-serializable `IntermediateOutput` component.
- [ ] Custom commands / cmd flags
  - [x] Inspect variable
    - [x] Inspect raw stack
  - [x] Inspect memory vars
    - [x] Inspect raw memory
  - [x] Inspect storage vars
    - [ ] Inspect raw storage slots / storage layout
  - [ ] Inspection verbosity configuration
  - [ ] Undo
  - [ ] Inspect bytecode / mnenomic of local or remote contracts.
    - [ ] Possibly use the forge debugger for this?
  - [x] Fetch contract interface from Etherscan ABI
  - [ ] Import remote sources from GitHub
  - [x] Enable / disable call trace printing
    - [x] Rip trace printing code from another module of foundry.
  - [x] On-the-fly network forking
  - [x] Export to file
    - [x] Export session to script contract if within project.
- [x] [Syntax highlighting](https://docs.rs/rustyline/10.0.0/rustyline/highlight/trait.Highlighter.html)
- [x] Binary subcommands
- [x] Tests.
  - [x] Cache
- [x] Benchmarks.
  - [x] Session Source
    - [x] Building
    - [x] Executor
    - [x] Inspection
    - [x] Cloning
- [ ] Optimizations (after MVP).
  - [ ] Speed up REPL execution time.
    - [ ] Use flamegraph to determine plan of attack.
    - [ ] Rework SessionSource clone, does not need to be a full deep copy.
    - [ ] Cache the backend within the executor so that it is not regenerated on each run. This causes lag with forks especially. We should keep the option to refresh each time to keep live state, but disable this by default.
- [ ] Finish README.
  - [ ] Examples
  - [ ] Migration from existing REPLs
- [ ] First review.
  - [x] Support ENV var interpolation in fork urls
  - [x] Allow named sessions
  - [x] Rename `!flush` to `!save`
  - [x] Check fork URL validity
  - [x] Add builtin command shorthands
  - [ ] ...

## Why?

Ever wanted to quickly test a small feature in solidity?

Perhaps to test how custom errors work, or how to write inline assembly?

Chisel is a fully-functional Solidity REPL, allowing you to write, execute, and debug Solidity directly in the command line.

Once you finish testing, Chisel even lets you export your code to a new solidity file!

In this sense, Chisel even serves as a Foundry script generator.

## Feature Completion

[soli](https://github.com/jpopesculian/soli) and [solidity-shell](https://github.com/tintinweb/solidity-shell) both provide a great solidity REPL, achieving:

- Statement support
- Custom events, errors, functions, imports
- Inspecting variables
- Forking remote chains
- Session caching

Chisel aims to improve upon existing Solidity REPLs by integrating with foundry as well as offering additional functionality:

- More verbose variable / state inspection
- Improved error messages
- Foundry-style call traces
- In-depth environment configuration
- ... and many more future features!

### Migrating from [soli](https://github.com/jpopesculian/soli) or [solidity-shell](https://github.com/tintinweb/solidity-shell)

_TODO_

## Installation

`chisel` is installed alongside Foundry cli commands!

Simply run `foundryup` to install `chisel`!

If you do not have `foundryup` installed, reference the Foundry [installation guide](../README.md#installation).

## Usage

### REPL Commands

```text
⚒️ Chisel help
=============
General
        !help | !h - Display all commands
        !exec <command> [args] | !e <command> [args] - Execute a shell command and print the output

Session
        !clear | !c - Clear current session source
        !source | !so - Display the source code of the current session
        !save [id] | !s [id] - Save the current session to cache
        !load <id> | !l <id> - Load a previous session ID from cache
        !list | !ls - List all cached sessions
        !clearcache | !cc - Clear the chisel cache of all stored sessions
        !export | !ex - Export the current session source to a script file
        !fetch <addr> <name> | !fe <addr> <name> - Fetch the interface of a verified contract on Etherscan

Environment
        !fork <url> | !f <url> - Fork an RPC for the current session. Supply 0 arguments to return to a local network
        !traces | !t - Enable / disable traces for the current session

Debug
        !memdump | !md - Dump the raw memory of the current state
        !stackdump | !sd - Dump the raw stack of the current state
```

### Cache Session

While chisel sessions are not persistent by default, they can be saved to the cache via the builtin `save` command from within the REPL.

Sessions can also be named by supplying a single argument to the `save` command, i.e. `!save my_session`.

```text
$ chisel
➜ uint a = 1;
➜ uint b = a << 0x08;
➜ !save
Saved session to cache with ID = 0.
```

### Loading a Previous Session

Chisel allows you to load a previous session from your history.

To view your history, you can run `chisel list` or `!list`. This will print a list of your previous sessions, identifiable by their index.

You can also run `chisel view <id>` or `!view <id>` to view the contents of a specific session.

To load a session, run `chisel load <id>` or use the `!load <id>` where `<id>` is a valid session index (eg 2 in the example below).

```text
$ chisel list
⚒️ Chisel Sessions
"2022-10-27 14:46:29" - chisel-0.json
"2022-10-27 14:46:29" - chisel-1.json
$ chisel view 1
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.17;

contract REPL {
    event KeccakEvent(bytes32 hash);

    function run() public {
      emit KeccakEvent(keccak256(abi.encode("Hello, world!")));
    }
}
$ chisel load 1
➜ ...
```

### Clearing the Cache

To clear Chisel's cache (stored in `~/.foundry/cache/chisel`), use the `chisel clear-cache` or `!clearcache` command.

```text
➜ !clearcache
Cleared chisel cache!
```

### Toggling Traces

By default, traces will only be shown if an input causes the call to the REPL contract to revert. To turn traces on
regardless of the call result, use the `!traces` command or pass in a verbosity option of any level (`-v<vvvv>`) to
the chisel binary.

```text
➜ uint a
➜ contract Test {
    function get() external view returns (uint) {
       return 256;
    }
}
➜ Test t = new Test()
➜ !traces
Successfully enabled traces!
➜ a = t.get()
Traces:
  [69808] 0xBd770416a3345F91E4B34576cb804a576fa48EB1::run()
    ├─ [36687] → new <Unknown>@0xf4D9599aFd90B5038b18e3B551Bc21a97ed21c37
    │   └─ ← 183 bytes of code
    ├─ [315] 0xf4D9599aFd90B5038b18e3B551Bc21a97ed21c37::get() [staticcall]
    │   └─ ← 0x0000000000000000000000000000000000000000000000000000000000000100
    └─ ← ()

➜ a
Type: uint
├ Hex: 0x100
└ Decimal: 256
```

### Forking a Network

To fork a network within your chisel session, use the `!fork <rpc-url>` command or supply a `--fork-url <url>` flag
to the chisel binary. The `!fork` command also accepts aliases from the `[rpc_endpoints]` section of your `foundry.toml`
if chisel was launched in the root of a foundry project (ex. `!fork mainnet`), as well as interpolated environment variables
(ex. `!fork https://https://eth-mainnet.g.alchemy.com/v2/${ALCHEMY_KEY}`).

### Fetching an Interface of a Verified Contract

To fetch an interface of a verified contract on Etherscan, use the `!fetch` / `!f` command.

> *Note*
> At the moment, only contracts that are deployed and verified on mainnet can be fetched. Support for other
> networks with Etherscan explorers coming soon.

```text
➜ !fetch 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2 IWETH
Added 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2's interface to source as `IWETH`
```

### Executing a Shell Command

Shell commands can be executed within Chisel with the `!exec` / `!e` command.

```text
➜ !e ls
anvil
binder
Cargo.lock
Cargo.toml
cast
chisel
cli
common
config
CONTRIBUTING.md
Dockerfile
docs
evm
fmt
forge
foundryup
LICENSE-APACHE
LICENSE-MIT
README.md
rustfmt.toml
target
testdata
ui
utils
```
