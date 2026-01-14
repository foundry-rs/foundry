# Track Plan: Refactor Source Instrumentation for Main Repo Standards

## Phase 1: Engine and AST Grooming
- [x] Task: Refactor `Instrumenter` AST visitor for robust control flow and remove redundant comments (c190a74)
- [x] Task: Clean up `analysis.rs` and `lib.rs` in `evm/coverage` for ID stability and idiomatic patterns (e5f40a6)
- [ ] Task: Conductor - User Manual Verification 'Phase 1: Engine and AST Grooming' (Protocol in workflow.md)

## Phase 2: Pipeline Unification and CLI Grooming
- [ ] Task: Unify `CoverageArgs::run` pipeline using `tempfile` and `ProjectPathsConfig`
- [ ] Task: Groom `cmd/coverage.rs` for consistent indentation and idiomatic error handling
- [ ] Task: Ensure all executor modifications (`fuzz`, `invariant`) are clean and idiomatic
- [ ] Task: Conductor - User Manual Verification 'Phase 2: Pipeline Unification and CLI Grooming' (Protocol in workflow.md)

## Phase 3: Final Standards Check
- [ ] Task: Verify all tests in `coverage_instrumented.rs` pass with groomed code
- [ ] Task: Run `cargo fmt` and `cargo clippy` on modified crates only
- [ ] Task: Conductor - User Manual Verification 'Phase 3: Final Standards Check' (Protocol in workflow.md)
