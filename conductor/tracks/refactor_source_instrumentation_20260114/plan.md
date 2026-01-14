# Track Plan: Refactor Source Instrumentation for Main Repo Standards

## Phase 1: Engine and AST Grooming [checkpoint: 26bb4f9]
- [x] Task: Refactor `Instrumenter` AST visitor for robust control flow and remove redundant comments (c190a74)
- [x] Task: Clean up `analysis.rs` and `lib.rs` in `evm/coverage` for ID stability and idiomatic patterns (e5f40a6)
- [x] Task: Conductor - User Manual Verification 'Phase 1: Engine and AST Grooming' (Protocol in workflow.md) (26bb4f9)

## Phase 2: Pipeline Unification and CLI Grooming [checkpoint: 8614eef]
- [x] Task: Unify `CoverageArgs::run` pipeline using `tempfile` and `ProjectPathsConfig` (095e2a4)
- [x] Task: Groom `cmd/coverage.rs` for consistent indentation and idiomatic error handling (651927b)
- [x] Task: Ensure all executor modifications (`fuzz`, `invariant`) are clean and idiomatic (e730588)
- [x] Task: Conductor - User Manual Verification 'Phase 2: Pipeline Unification and CLI Grooming' (Protocol in workflow.md) (8614eef)

## Phase 3: Final Standards Check
- [x] Task: Verify all tests in `coverage_instrumented.rs` pass with groomed code (22707f4)
- [x] Task: Run `cargo fmt` and `cargo clippy` on modified crates only (651927b)
- [ ] Task: Conductor - User Manual Verification 'Phase 3: Final Standards Check' (Protocol in workflow.md)
