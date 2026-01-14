# Track Plan: Source Instrumentation for Forge Coverage

## Phase 1: Foundation and AST Analysis [checkpoint: f99f847]
- [x] Task: Research `solar` AST structures for Statements and Branches 88895
- [x] Task: Implement a basic AST visitor to identify coverage points cd6299f
- [x] Task: Conductor - User Manual Verification 'Phase 1: Foundation and AST Analysis' (Protocol in workflow.md)

## Phase 2: Instrumentation Engine [checkpoint: d0ee454]
- [x] Task: Develop a source-to-source rewriter using `solar` b64175b
- [x] Task: Implement injection logic for `vm.coverageHit` calls 1166d77
- [x] Task: Handle edge cases (modifiers, inheritance, complex expressions) d0ee454
- [x] Task: Conductor - User Manual Verification 'Phase 2: Instrumentation Engine' (Protocol in workflow.md)

## Phase 3: Runtime and Integration [checkpoint: aad657f]
- [x] Task: Implement `coverageHit` cheatcode handling in `revm` inspectors 54f16e3
- [x] Task: Add `--instrument-source` flag to `forge coverage` CLI 54f16e3
- [x] Task: Update `Instrumenter` to emit `CoverageItem`s 54f16e3
- [x] Task: Modify `forge` build pipeline to use instrumented sources in a temp directory aad657f
- [x] Task: Conductor - User Manual Verification 'Phase 3: Runtime and Integration' (Protocol in workflow.md)

## Phase 4: Reporting and Polish
- [ ] Task: Debug runtime interception of `vm.coverageHit` (Currently fails with unknown cheatcode)
- [ ] Task: Adapt `CoverageReport` to handle hits from instrumented source (Partially done via SourceAnalysis construction)
- [ ] Task: Verify accuracy against the existing bytecode tracing method
- [ ] Task: Test with "Stack Too Deep" contracts to ensure resolution
- [ ] Task: Conductor - User Manual Verification 'Phase 4: Reporting and Polish' (Protocol in workflow.md)