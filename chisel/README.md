# `chisel`

Chisel is a fast, utilitarian, and verbose solidity REPL.

## Why?

Ever wanted to quickly test a small feature in solidity?

Perhaps to test how custom errors work, or how to write inline assembly?

Chisel is your solution. Chisel let's you write, execute, and debug Solidity directly in the command line.

Once you finish testing, Chisel even lets you export your code to a new solidity file!

In this sense, Chisel even serves as a project generator.



## Checklist

- [ ] REPL functionality
  - [ ] Create temporary REPL contract (in memory, or temp file?).
    - [ ] Implement `forge-std/Test.sol` so that cheatcodes etc. can be used.
  - [ ] Utilize foundry's `evm` module for REPL env.
    - [ ] Implement network forking.
  - [ ] Import all project files / interfaces into REPL contract automatically.
    - [ ] Optionally disable this functionality with a flag.
- [ ] Cache REPL History
  - [ ] Allow a user to save/load sessions from their Chisel history.
- [ ] Custom commands / cmd flags
  - [ ] Inspect variable
  - [ ] Inspect memory
  - [ ] Inspect storage slot
  - [ ] Enable / disable call tracing
  - [ ] Network forking
  - [ ] Export to file
- [ ] [Syntax highlighting](https://docs.rs/rustyline/10.0.0/rustyline/highlight/trait.Highlighter.html)
- [ ] Tests.
