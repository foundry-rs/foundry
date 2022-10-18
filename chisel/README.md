# `chisel`

Chisel is a fast, utilitarian, and verbose solidity REPL. It is heavily inspired by the incredible work done in [soli](https://github.com/jpopesculian/soli)!

## Why?

Ever wanted to quickly test a small feature in solidity?

Perhaps to test how custom errors work, or how to write inline assembly?

Chisel is your solution. Chisel let's you write, execute, and debug Solidity directly in the command line.

Once you finish testing, Chisel even lets you export your code to a new solidity file!

In this sense, Chisel even serves as a project generator.

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
    - [ ] Implement `forge-std/Test.sol` so that cheatcodes etc. can be used.
  - [x] Utilize foundry's `evm` module for REPL env.
    - [x] Implement network forking.
  - [ ] Import all project files / interfaces into REPL contract automatically.
    - [ ] Optionally disable this functionality with a flag.
- [x] Cache REPL History
  - [x] Allow a user to save/load sessions from their Chisel history.
- [ ] Custom commands / cmd flags
  - [x] Inspect variable
  - [x] Inspect memory
  - [ ] Inspect storage slot
  - [ ] Enable / disable call tracing
    - [ ] Rip trace printing code from another module of foundry.
  - [x] On-the-fly network forking
  - [x] Export to file
    - [ ] Export session to script contract if within project.
- [x] [Syntax highlighting](https://docs.rs/rustyline/10.0.0/rustyline/highlight/trait.Highlighter.html)
- [ ] Tests.

## Usage

### Installation

`chisel` is installed alongside Foundry cli commands!

Simply run `foundryup` to install `chisel`!

If you do not have `foundryup` installed, reference the Foundry [installation guide](../README.md#installation).

### Project Generation

Below is an example of how to use `chisel` to generate a new project.

```bash
$ chisel
chisel > constructor() ERC20("NewProject", "NEWP", 18) {}
Compilation Successful!
chisel > exit
$ chisel generate --name new_erc20
Exporting...
Generated new project "new_erc20"!
$ ls
new_erc20
```

### Loading a Previous Session

Chisel allows you to load a previous session from your history.

To view your history, you can run `chisel history`. This will print a list of your previous sessions, identifiable by their index.

You can also run `chisel view <index>` to view the contents of a specific session.

To load a session, run `chisel load <index>` where `<index>` is a valid session index (eg 2 in the example below).

```bash
$ chisel history
1. 2022-05-06 15:04:32
2. 2022-02-23 07:43:12
$ chisel view 2
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.3;

contract REPL {
    event KeccakEvent(bytes32 hash);

    constructor() ERC20("Mock ERC20", "MERC", 18) {}

    function testFunction() public {
      emit KeccakEvent(keccak256(abi.encode("Hello, world!")));
    }
}
$ chisel load 2
chisel > ...
```
