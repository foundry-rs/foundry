# Track Spec: Source Instrumentation for Forge Coverage

## Objective
Implement a source-code instrumentation mode for `forge coverage` that allows running coverage on optimized code, thereby resolving the "Stack Too Deep" errors caused by disabling optimizations.

## Background
Currently, `forge coverage` relies on bytecode tracing and compiler source maps. This requires disabling optimizations to maintain map accuracy. However, complex contracts often fail to compile without optimizations due to EVM stack limits. Hardhat's `solidity-coverage` solves this by injecting counters directly into the source code, allowing the optimizer to run.

## Requirements
- Support Statement and Branch coverage.
- Allow compilation with full optimizations enabled.
- Integrate seamlessly into the existing `forge coverage` command.
- Use `solar` for AST analysis and instrumentation.

## Architecture
1. **Instrumenter:** Parses Solidity source, identifies coverage items, and injects tracing logic.
2. **Hit Collector:** A mechanism (likely a custom cheatcode/event) to record hits during execution.
3. **Reporter:** Maps the collected hits back to the original source lines and branches.

## Proposed Hit Mechanism
Inject calls to a internal-only cheatcode:
`vm.coverageHit(uint32 itemId)`
The REVM inspector will intercept this call and increment the corresponding counter.
