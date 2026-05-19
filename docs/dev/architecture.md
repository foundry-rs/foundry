# Architecture

This document describes the high-level architecture of Foundry.

## Binaries

### `forge/`

The `forge` binary — Foundry's fast and flexible Ethereum testing framework. Integrates
compilation, testing, fuzzing, gas snapshots, coverage, deployment scripting, and contract
verification into a single CLI tool.

### `cast/`

The `cast` binary — a command-line tool for performing Ethereum RPC calls, encoding/decoding ABI
data, querying on-chain state, and sending transactions.

### `anvil/`

The `anvil` binary — a fast local Ethereum development node. Supports instant mining, mainnet
forking, custom gas limits, state snapshots, and a JSON-RPC interface compatible with Hardhat and
Ganache.

### `chisel/`

The `chisel` binary — a fast, utilitarian Solidity REPL. Allows writing and executing Solidity
expressions interactively, with built-in inspection of EVM state and gas usage.

## Core Libraries

### `evm/`

Foundry's EVM tooling. This is built around [`revm`](https://github.com/bluealloy/revm) and has additional
implementation of:

- [cheatcodes](./cheatcodes.md): a set of solidity calls dedicated to testing which can manipulate the environment in which the execution is run

### `cheatcodes/`

Definitions and implementations of all Foundry cheatcodes — the `vm.*` Solidity calls used in
tests and scripts to manipulate EVM state, mock external calls, record events, deal tokens, and
more. The cheatcodes specification lives in a companion crate (`foundry-cheatcodes-spec`) so it can
be consumed without pulling in the full implementation.

### `config/`

Includes all of Foundry's settings and how to get them. Reads `foundry.toml`, environment
variables, and CLI flags, then merges them into a resolved `Config` struct that every other crate
consumes.

### `common/`

Shared utilities used across all Foundry crates: ABI helpers, provider construction, file I/O,
formatting, selectors, and more. Anything needed by two or more other crates typically ends up here.

### `primitives/`

Low-level Foundry-specific types: network abstractions and transaction envelope types that bridge
alloy/revm primitives to Foundry's domain model.

## CLI & TUI

### `cli/`

The core `forge` and `cast` cli implementation. Includes all subcommands. Contains argument
parsing (via `clap`) and the top-level command dispatch that delegates to the crate owning each
feature.

### `cli-markdown/`

A utility crate that generates Markdown documentation for clap-based CLIs. Used to produce the
auto-generated command reference pages for the Foundry book.

### `debugger/`

The interactive Solidity TUI debugger. Renders execution traces in a terminal UI (via `ratatui`),
allows stepping through opcodes, and can dump debugger-compatible JSON files for offline analysis.

## Compilation & Formatting

### `fmt/`

The Solidity code formatter. Parses Solidity source and re-emits it following the
[Solidity style guide](https://docs.soliditylang.org/en/latest/style-guide.html). Tested against
the Prettier Solidity plugin's test suite.

### `lint/`

The Solidity linter. Performs static analysis to identify potential errors, vulnerabilities, gas
inefficiencies, and style guide violations across a project's Solidity source files.

### `doc/`

The Solidity documentation generator. Parses NatSpec comments from Solidity source and emits
Markdown or mdBook output for publishing contract documentation.

### `linking/`

Smart contract linking tools. Resolves library placeholders in compiled bytecode by matching
library names to their deployed addresses, handling both pre-known and at-deploy-time addresses.

### `sol-macro-gen/`

Generates Rust bindings from Solidity ABI definitions using the `sol!` procedural macro. Used by
the `forge bind` and `forge bind-json` subcommands.

## Scripting & Deployment

### `script/`

The Solidity scripting engine behind `forge script`. Executes deployment scripts in a simulated or
live EVM environment, collects broadcast transactions, and coordinates multi-chain deployments.

### `script-sequence/`

Types that represent a broadcast sequence produced by `forge script`: the ordered list of
transactions, their receipts, and metadata needed to resume or replay a deployment.

### `verify/`

Smart contract verification tools. Submits compiled source and metadata to Etherscan, Blockscout,
Sourcify, and other verification backends, then polls until verification succeeds or fails.

## Testing & Internal Utilities

### `macros/`

Internal Foundry procedural macros. Provides derive and attribute macros used within the Foundry
codebase itself (not exported for end users).

### `test-utils/`

Foundry's internal testing utilities. Provides helpers for spawning test processes, asserting
command output, and setting up fixture projects — used by integration tests across the workspace.
