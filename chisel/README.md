# Chisel

Chisel is a fast, utilitarian, and verbose solidity REPL.

_TODO_

## Checklist

- [ ] REPL functionality
  - [ ] Create temporary REPL contract (in memory, or temp file?).
    - [ ] Implement `forge-std/Test.sol` so that cheatcodes etc. can be used.
  - [ ] Utilize foundry's `evm` module for REPL env.
    - [ ] Implement network forking.
  - [ ] Import all project files / interfaces into REPL contract automatically.
    - [ ] Optionally disable this functionality with a flag.
- [ ] Custom commands / cmd flags
  - [ ] Inspect variable
  - [ ] Inspect memory
  - [ ] Inspect storage slot
  - [ ] Enable / disable call tracing
  - [ ] Network forking
- [ ] [Syntax highlighting](https://docs.rs/rustyline/10.0.0/rustyline/highlight/trait.Highlighter.html)
