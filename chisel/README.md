# `chisel`

Chisel is a fast, utilitarian, and verbose solidity REPL. It is heavily inspired by the incredible work done in [soli](https://github.com/jpopesculian/soli)!

## Why?

Ever wanted to quickly test a small feature in solidity?

Perhaps to test how custom errors work, or how to write inline assembly?

Chisel is a fully-functional Solidity REPL, allowing you to write, execute, and debug Solidity directly in the command line.

Once you finish testing, Chisel even lets you export your code to a new solidity file!

In this sense, Chisel even serves as a Foundry script generator.

## Feature Completion

[soli](https://github.com/jpopesculian/soli) provides a great solidity REPL, achieving:

- Statements
- Custom events, errors, functions, imports
- Inspecting a variable

Chisel aims to improve upon soli, with native foundry integration by providing feature completion with:

- Fork an existing chain
- More advanced introspection
- Better error messages and traces
- ... many more future features!

## Checklist

- [ ] REPL functionality
  - [x] Create temporary REPL contract (in memory, or temp file?).
    - [x] Implement `forge-std/Test.sol` so that cheatcodes, etc. can be used.
  - [x] Utilize foundry's `evm` module for REPL env.
    - [x] Implement network forking.
  - [ ] ~~Expression evaluation / inspection (i.e. the input `0x01 << 0x08` should inspect a `uint` of value `256`)~~
    - Nixed due to type ambiguity. If displaying bytes is sufficient, we can still do this.
  - [x] Input history.
  - [ ] Use forge fmt module to format source code when printing via the `!source` command. (?)
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
- [ ] Optimizations.
  - [ ] Speed up REPL execution time.
- [x] Finish README.
- [ ] First review.
  - [ ] *add requested changes here*

## Usage

### Installation

`chisel` is installed alongside Foundry cli commands!

Simply run `foundryup` to install `chisel`!

If you do not have `foundryup` installed, reference the Foundry [installation guide](../README.md#installation).

### Cache Session

While chisel sessions are not persistent by default, they can be saved to the cache via the builtin `flush` command from within the REPL.

```bash
$ chisel
➜ uint a = 1;
➜ uint b = a << 0x08;
➜ !flush
Saved session to cache with ID = 0.
```

### Loading a Previous Session

Chisel allows you to load a previous session from your history.

To view your history, you can run `chisel list` or `!list`. This will print a list of your previous sessions, identifiable by their index.

You can also run `chisel view <id>` or `!view <id>` to view the contents of a specific session.

To load a session, run `chisel load <id>` or use the `!load <id>` where `<id>` is a valid session index (eg 2 in the example below).

```bash
$ chisel history
1. 2022-05-06 15:04:32 - chisel-0.json
2. 2022-02-23 07:43:12 - chisel-1.json
$ chisel view 2
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.17;

contract REPL {
    event KeccakEvent(bytes32 hash);

    function run() public {
      emit KeccakEvent(keccak256(abi.encode("Hello, world!")));
    }
}
$ chisel load 2
➜ ...
```

### Clearing the Cache

To clear Chisel's cache (stored in `~/.foundry/cache/chisel`), use the `chisel clearcache` or `!clearcache` commands.

```bash
➜ !clearcache
Cleared chisel cache!
```

### Toggling Traces

By default, traces will only be shown if an input causes the call to the REPL contract to revert. To turn traces on
regardless of the call result, use the `!traces` command or pass in a verbosity option of any level (`-v<vvvv>`) to
the chisel binary.

```bash
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

To fork a network within your chisel session, use the `!fork <rpc-url>` command or supply a `--fork-url <rpc-url>` flag
to the chisel binary. The `!fork` command also accepts aliases from the `[rpc_endpoints]` section of your `foundry.toml`,
if chisel was launched in the root of a foundry project (ex. `!fork mainnet`).
