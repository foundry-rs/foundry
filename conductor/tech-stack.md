# Technology Stack

## Core Technologies
- **Rust:** The primary programming language used for the entire toolkit, chosen for its performance, safety, and modern tooling.
- **Alloy:** A high-performance, modular Ethereum library for interacting with the EVM, sending transactions, and handling types.
- **REVM:** A modular and fast EVM implementation written in Rust, used as the execution engine for tests and simulation.
- **Cargo:** Rust's package manager and build system, used to manage dependencies and build the workspace.

## Compiler & Analysis
- **Solar:** A Solidity compiler and analysis framework used for advanced parsing and semantic analysis.
- **Foundry-Compilers:** A library for managing multiple versions of the Solidity and Vyper compilers.

## Infrastructure & Concurrency
- **Tokio:** An asynchronous runtime for Rust, used for high-performance I/O and task management.
- **Rayon:** A data-parallelism library for Rust, used to parallelize compilation and test execution.
- **Serde:** A framework for serializing and deserializing Rust data structures efficiently.

## Testing & Quality
- **Proptest:** A property-based testing framework used to find edge cases in contract logic.
- **CLI Utilities:** Clap for command-line argument parsing, Indicatif for progress bars, and Comfy-table for formatted output.
