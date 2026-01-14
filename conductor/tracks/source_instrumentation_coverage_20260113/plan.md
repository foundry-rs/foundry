# Track Plan: Source Instrumentation for Forge Coverage

## Phase 1: Foundation and AST Analysis [checkpoint: f99f847]
- [x] Task: Research `solar` AST structures for Statements and Branches 88895
- [x] Task: Implement a basic AST visitor to identify coverage points cd6299f
- [x] Task: Conductor - User Manual Verification 'Phase 1: Foundation and AST Analysis' (Protocol in workflow.md)

## Phase 2: Instrumentation Engine
- [ ] Task: Develop a source-to-source rewriter using `solar`
- [ ] Task: Implement injection logic for `vm.coverageHit` calls
- [ ] Task: Handle edge cases (modifiers, inheritance, complex expressions)
- [ ] Task: Conductor - User Manual Verification 'Phase 2: Instrumentation Engine' (Protocol in workflow.md)

## Phase 3: Runtime and Integration
- [ ] Task: Implement `coverageHit` cheatcode handling in `revm` inspectors
- [ ] Task: Add `--instrument-source` flag to `forge coverage` CLI
- [ ] Task: Modify `forge` build pipeline to use instrumented sources in a temp directory
- [ ] Task: Conductor - User Manual Verification 'Phase 3: Runtime and Integration' (Protocol in workflow.md)

## Phase 4: Reporting and Polish
- [ ] Task: Adapt `CoverageReport` to handle hits from instrumented source
- [ ] Task: Verify accuracy against the existing bytecode tracing method
- [ ] Task: Test with "Stack Too Deep" contracts to ensure resolution
- [ ] Task: Conductor - User Manual Verification 'Phase 4: Reporting and Polish' (Protocol in workflow.md)
