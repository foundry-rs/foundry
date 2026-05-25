# Architecture

This document describes the high-level architecture of Foundry.
All crates live under `crates/` in the workspace.

---

## User-facing tools

### `forge/`

Fast and flexible Ethereum testing framework. Handles:

- Test discovery, execution, and gas snapshots
- Contract compilation, deployment, and size checks
- Coverage reporting
- Invariant / fuzz / symbolic testing
- Contract verification (delegates to [`verify/`](#verify))
- Bytecode comparison

Entry point: `forge` binary.

### `cast/`

Command-line tool for interacting with Ethereum nodes and contracts.
Supports RPC calls, ABI encoding/decoding, transaction signing and sending,
ENS resolution, unit conversion, and more.

Entry point: `cast` binary.

### `anvil/`

A local Ethereum node for development and testing.  Built on top of
[`evm/`](#evm) and exposes a full JSON-RPC server (including `debug_*`,
`trace_*`, and `anvil_*` namespaces).  Supports forking mainnet/testnets,
time-travel cheatcodes, and state save/load.

Entry point: `anvil` binary.

### `chisel/`

Fast, utilitarian, and verbose Solidity REPL.  Lets you write and evaluate
Solidity snippets interactively, inspect results, and step through execution
without a full test harness.

Entry point: `chisel` binary.

---

## Core EVM & configuration

### `evm/`

Foundry's EVM tooling.  Built around [`revm`](https://github.com/bluealloy/revm)
and adds:

- [cheatcodes](./cheatcodes.md): a set of Solidity calls dedicated to testing
  that can manipulate the EVM environment (fork, warp, prank, snapshot, …)
- Execution inspectors for traces, coverage, and call graphs
- Backend helpers for in-process forking

### `config/`

All of Foundry's settings and how to resolve them.  Reads `foundry.toml`,
environment variables, and per-project overrides; exposes a single `Config`
struct used by every tool.

---

## Common utilities

### `common/`

Shared utilities used across multiple Foundry crates:

- ABI/selector helpers
- Compile and artifact caching
- Etherscan/explorer API wrappers
- Shell formatting (color, tables)
- File and path helpers

### `primitives/`

Low-level Foundry types shared across crates:

- `FoundryNetwork` — named-network enum with chain-ID constants
- Transaction type helpers

### `cli/`

Common CLI infrastructure: argument parsing helpers built on
[`clap`](https://github.com/clap-rs/clap), global option structs, and
shell-completion utilities reused by `forge`, `cast`, `anvil`, and `chisel`.

### `cli-markdown/`

Generates Markdown documentation from [`clap`](https://github.com/clap-rs/clap)
`Command` trees.  Used by the CI pipeline that auto-publishes Foundry's
reference docs to [the book](https://book.getfoundry.sh/).

---

## Source-code processing

### `fmt/`

Solidity code formatter.  Built on top of
[Solar](https://github.com/paradigmxyz/solar) and implemented with a
Wadler-style pretty-printing engine.  Exposed as `forge fmt`.

### `lint/`

Solidity linter that identifies:

- High-severity vulnerabilities (unsafe external calls, reentrancy patterns, …)
- Gas optimizations
- Style-guide violations

Exposed as `forge lint`.

### `doc/`

Solidity documentation generator.  Parses NatSpec comments and produces
Markdown or HTML output.  Exposed as `forge doc`.

---

## Scripting & debugging

### `script/`

Solidity scripting support (the runtime side of `forge script`).  Handles:

- Running user-written Solidity scripts against a live or forked network
- Gas estimation, simulation, and broadcast
- Multi-sender / hardware-wallet flows

### `script-sequence/`

Types for script broadcast sequences (the JSON files written to
`broadcast/`).  Shared between `script/` (write) and tools that replay or
inspect past broadcasts (read).

### `debugger/`

Interactive TUI debugger for Solidity execution.  Lets you step through EVM
opcodes, inspect the stack/memory/storage, and view source-mapped Solidity
lines.  Also supports dumping debugger state to a file for offline analysis.

Exposed as `forge debug`.

---

## Linking & verification

### `linking/`

Smart-contract linker.  Resolves library placeholder addresses inside compiled
bytecode before deployment, handling both auto-deploy and user-supplied library
address flows.

### `verify/`

Contract verification tools.  Encodes and submits verification requests to
Etherscan, Sourcify, Blockscout, and other explorers.  Normalizes source maps,
handles multi-file and single-file flattening, and polls for verification status.

Exposed as `forge verify-contract` and `forge verify-bytecode`.

---

## Code generation & macros

### `sol-macro-gen/`

Generates Rust bindings (structs, functions, events) from Solidity ABIs using
the `sol!` macro.  Used internally by `alloy-sol-macro` integration and by
Foundry's own ABI helpers.

### `macros/`

Internal procedural macros used only within the Foundry workspace:

- `#[cheatcode]` — registers a cheatcode handler in `evm/`
- Console-format helpers for `console.log` emulation

---

## Testing infrastructure

### `test-utils/`

Shared test helpers and fixtures used in Foundry's own test suite:

- Temporary directory utilities
- Pre-built project templates for integration tests
- RPC snapshot helpers

---

## Dependency map (simplified)

```
forge ──► evm, config, common, script, linking, verify, debugger, doc, fmt, lint
cast  ──► evm, config, common
anvil ──► evm, config, common
chisel──► evm, config, common, fmt
          │
          └─ evm ──► primitives
          └─ config ──► primitives, common
          └─ common ──► primitives
          └─ script ──► evm, common, script-sequence
          └─ verify ──► common, config
          └─ debugger──► evm, common
          └─ fmt ──► (Solar parser)
          └─ lint ──► (Solar parser)
```

> **Note:** This diagram reflects high-level relationships; see individual
> `Cargo.toml` files for the authoritative dependency list.
