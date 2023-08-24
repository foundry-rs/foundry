# `chisel`

Chisel is a fast, utilitarian, and verbose Solidity REPL. It is heavily inspired by the incredible work done in [soli](https://github.com/jpopesculian/soli) and [solidity-shell](https://github.com/tintinweb/solidity-shell)!

![preview](./assets/preview.gif)

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

Migration from existing Solidity REPLs such as [soli](https://github.com/jpopesculian/soli) or [solidity-shell](https://github.com/tintinweb/solidity-shell) is as
simple as installing Chisel via `foundryup`. For information on features, usage, and configuration, see the [Usage](#usage) section as well as the chisel manpage (`man chisel` or `chisel --help`).

## Installation

To install `chisel`, simply run `foundryup`!

If you do not have `foundryup` installed, reference the Foundry [installation guide](../../README.md#installation).

## Usage

### REPL Commands

```text
⚒️ Chisel help
=============
General
        !help | !h - Display all commands
        !quit | !q - Quit Chisel
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
        !edit - Open the current session in an editor

Environment
        !fork <url> | !f <url> - Fork an RPC for the current session. Supply 0 arguments to return to a local network
        !traces | !t - Enable / disable traces for the current session
        !calldata [data] | !cd [data] - Set calldata (`msg.data`) for the current session (appended after function selector). Clears it if no argument provided.

Debug
        !memdump | !md - Dump the raw memory of the current state
        !stackdump | !sd - Dump the raw stack of the current state
        !rawstack <var> | !rs <var> - Display the raw value of a variable's stack allocation. For variables that are > 32 bytes in length, this will display their memory pointer.
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
(ex. `!fork https://eth-mainnet.g.alchemy.com/v2/${ALCHEMY_KEY}`).

### Fetching an Interface of a Verified Contract

To fetch an interface of a verified contract on Etherscan, use the `!fetch` / `!f` command.

> **Note**
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
