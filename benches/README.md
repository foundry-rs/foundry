# Foundry Benchmarks

Performance benchmarks for Foundry, comparing a **baseline** version against a **feature** version
across real-world Solidity repositories. Produces structured JSON output with automated regression
detection.

## Prerequisites

1. **Rust and Cargo** — for building the benchmark binary
2. **Foundryup** — `curl -L https://foundry.paradigm.xyz | bash && foundryup`
3. **[Hyperfine](https://github.com/sharkdp/hyperfine)** — the benchmarking harness
4. **Git** — for cloning test repositories
5. **Node.js and npm** — some test repos have npm dependencies

## Quick Start

```bash
# Build the benchmark runner
cargo build --release --bin foundry-bench

# Compare stable vs nightly using the built-in bench-suite (default)
./target/release/foundry-bench --baseline stable --feature nightly --force-install

# Output structured JSON for automation
./target/release/foundry-bench --baseline stable --feature nightly --json

# Compare specific branches/commits
./target/release/foundry-bench --baseline v1.3.6 --feature v1.4.0 --force-install

# Run specific benchmarks
./target/release/foundry-bench \
  --baseline stable --feature nightly \
  --benchmarks forge_test,forge_fuzz_test,forge_invariant_test,forge_fork_test \
  --fork-url https://eth.merkle.io

# Run on external repositories instead of the built-in suite
./target/release/foundry-bench \
  --baseline stable --feature nightly \
  --repos ithacaxyz/account:v0.3.2,Vectorized/solady:v0.1.22
```

## Benchmark Types

| Benchmark | Command | What It Measures |
|-----------|---------|-----------------|
| `forge_test` | `forge test` | Overall test execution speed |
| `forge_fuzz_test` | `forge test --match-test "test[^(]*\([^)]+\)"` | Fuzz test performance |
| `forge_invariant_test` | `forge test --match-test "invariant"` | Invariant test performance |
| `forge_cheatcode_test` | `forge test --match-contract CheatcodeTests` | Cheatcode inspector throughput |
| `forge_fork_test` | `forge test --match-contract ForkTests` | Fork-mode RPC reads/writes |
| `forge_multifork_test` | `forge test --match-contract MultiForkTests` | Multi-fork switching, persistent state |
| `forge_isolate_test` | `forge test --isolate` | Isolated test execution |
| `forge_build_no_cache` | `forge build` (cold) | Compilation speed without cache |
| `forge_build_with_cache` | `forge build` (warm) | Cache hit performance |
| `forge_coverage` | `forge coverage --ir-minimum` | Coverage analysis speed |

Default benchmarks (when `--benchmarks` is not specified): `forge_test`, `forge_fuzz_test`,
`forge_invariant_test`.

## Built-in Bench Suite

By default, `foundry-bench` uses the built-in benchmark suite at `benches/fixtures/bench-suite/`
instead of cloning external repositories. This suite is:

- **Self-contained**: no git submodules, no external dependencies, no forge-std
- **Backwards compatible**: `pragma solidity >=0.8.0`, stable across Foundry versions
- **Deterministic**: no network calls, no randomness beyond the fuzzer
- **Targeted**: contracts designed to stress specific Foundry subsystems

The suite includes:

| Contract | What it exercises |
|----------|------------------|
| `ERC20` | Minimal token — baseline EVM execution, storage reads/writes |
| `Vault` | AMM constant-product pool — math-heavy, multi-contract interactions |
| `Registry` | Mapping-heavy key-value store — storage-bound workloads, batch ops |

Test files:

| File | Type | What it benchmarks |
|------|------|--------------------|
| `UnitTests.t.sol` | Unit | Test runner startup / TTFB, basic assertion speed |
| `FuzzERC20.t.sol` | Fuzz | Fuzzer input generation, property checking |
| `FuzzVault.t.sol` | Fuzz | Multi-step fuzz with math validation |
| `InvariantVault.t.sol` | Invariant | Handler-based invariant testing, state exploration |
| `InvariantRegistry.t.sol` | Invariant | Storage-heavy invariant testing, multi-actor |
| `CheatcodeTests.t.sol` | Cheatcodes | Inspector throughput: deal, prank, warp, store, snapshot, mock |
| `ForkTests.t.sol` | Fork | `vm.createFork`, forked state reads/writes, `vm.rollFork` |
| `MultiForkTests.t.sol` | Multi-fork | Fork switching, `vm.makePersistent`, cross-fork reads |

To use external repos instead: `--repos ithacaxyz/account:v0.3.2,Vectorized/solady:v0.1.22`

## CLI Options

| Option | Description | Default |
|--------|-------------|---------|
| `--baseline <VERSION>` | Baseline Foundry version | `stable` |
| `--feature <VERSION>` | Feature Foundry version | `nightly` |
| `--benchmarks <LIST>` | Comma-separated benchmark names | test, fuzz, invariant |
| `--local` | Use built-in bench-suite fixtures | `true` |
| `--repos <LIST>` | External repos (`org/repo[:rev]`), disables `--local` | — |
| `--runs <N>` | Number of runs per benchmark | `5` |
| `--json` | Output structured JSON bundle | `false` |
| `--noise-threshold <PCT>` | Noise threshold for verdict (%) | `3.0` |
| `--fork-url <URL>` | RPC URL for fork/multifork benchmarks (required for those types) | — |
| `--force-install` | Force install versions before benchmarking | `false` |
| `--verbose` | Show hyperfine output | `false` |
| `--output-dir <DIR>` | Directory for output files | `.` |
| `--output-file <FILE>` | Markdown output filename | `LATEST.md` |

## Output Formats

### Markdown (default)

A comparison table showing baseline vs feature with delta percentages and verdicts:

```
## Forge Test

| Repository | Baseline | Feature | Delta | Verdict |
|------------|----------|---------|-------|---------|
| solady     | 2.28s    | 2.10s   | -7.9% | 🟢 improved |
```

### JSON (`--json`)

A structured bundle written to `bundle.json`:

```json
{
  "baseline_ref": "stable",
  "feature_ref": "nightly",
  "baseline_version": "forge 1.3.6 (d241588 2025-09-16)",
  "feature_version": "forge 1.4.0 (bd0e4a7 2025-10-01)",
  "timestamp": "2025-10-02T12:14:23Z",
  "system": { "os": "linux", "cpu_cores": 32, "rustc": "rustc 1.90.0" },
  "comparisons": [
    {
      "benchmark": "forge_test",
      "repo": "solady",
      "baseline_mean": 2.28,
      "feature_mean": 2.10,
      "delta_pct": -7.89,
      "verdict": "improved"
    }
  ],
  "overall_verdict": "improved"
}
```

## Verdict Logic

Each comparison is classified based on the delta percentage and `--noise-threshold`:

| Delta | Verdict |
|-------|---------|
| `< -threshold` | 🟢 improved |
| `> +threshold` | 🔴 regressed |
| within ±threshold | ⚪ neutral |

The overall verdict is **regressed** if any individual comparison regressed, **improved** if at
least one improved and none regressed, otherwise **neutral**.

The process exits with code 1 if the overall verdict is **regressed**.

## CI Integration

The GitHub Actions workflow (`.github/workflows/benchmarks.yml`) runs benchmarks on
`workflow_dispatch` and can post results as PR comments.

```bash
# CI usage: exit non-zero on regression
foundry-bench \
  --baseline stable --feature nightly \
  --json --noise-threshold 5.0
```

## Profiling

Build Foundry with the `profiling` profile for CPU profiling with samply:

```bash
# In the foundry repo
cargo build --profile profiling --bin forge

# Run under samply
samply record -- ./target/profiling/forge test

# Or use hyperfine with the profiling build
foundry-bench --baseline stable --feature nightly --verbose
```
