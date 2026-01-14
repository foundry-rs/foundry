# Track Spec: Refactor Source Instrumentation for Main Repo Standards

## Overview
This track focuses on refactoring the recently implemented source-to-source instrumentation engine for `forge coverage`. The goal is to eliminate "AI slop," unify fragmented architectural paths, and ensure the implementation is fully idiomatic according to the standards of the main Foundry repository.

## Functional Requirements
- **Unified Reporting Pipeline:** Integrate source instrumentation logic into the standard `prepare` and `collect` workflow in `CoverageArgs` to reduce duplication.
- **Idiomatic Source Discovery:** Leverage `ProjectPathsConfig` and `ProjectCompiler` for all source file discovery and iteration.
- **Standardized Temp Management:** Employ the `tempfile` crate for all temporary directory operations.
- **AST Visitor Refinement:** Refactor the `Instrumenter` visitor in `crates/forge/src/coverage/instrument.rs` to remove brittle control flow.
- **Code Quality Polish:** Resolve inconsistent indentation, redundant comments, and manual error handling.

## Scope (Files to Groom)
- `crates/evm/coverage/src/analysis.rs`
- `crates/evm/coverage/src/lib.rs`
- `crates/evm/coverage/src/source_inspector.rs`
- `crates/evm/evm/src/executors/fuzz/mod.rs`
- `crates/evm/evm/src/executors/fuzz/types.rs`
- `crates/evm/evm/src/executors/invariant/mod.rs`
- `crates/evm/evm/src/executors/invariant/replay.rs`
- `crates/evm/evm/src/executors/invariant/result.rs`
- `crates/evm/evm/src/executors/mod.rs`
- `crates/evm/evm/src/inspectors/stack.rs`
- `crates/evm/fuzz/src/lib.rs`
- `crates/forge/src/cmd/coverage.rs`
- `crates/forge/src/coverage/instrument.rs`
- `crates/forge/tests/cli/coverage_instrumented.rs`