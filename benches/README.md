# Foundry Benchmarks

This directory contains performance benchmarks for Foundry commands across multiple repositories and Foundry versions.

## Prerequisites

Before running the benchmarks, ensure you have the following installed:

1. **Rust and Cargo** - Required for building the benchmark binary

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **Foundryup** - The Foundry toolchain installer

   ```bash
   curl -L https://foundry.paradigm.xyz | bash
   foundryup
   ```

3. **Git** - For cloning benchmark repositories

4. [**Hyperfine**](https://github.com/sharkdp/hyperfine/blob/master/README.md) - The benchmarking tool used by foundry-bench

5. **Node.js and npm** - Some repositories require npm dependencies

## Running Benchmarks

Build and install the benchmark runner:

```bash
cargo build --release --bin foundry-bench
```

To install `foundry-bench` to your PATH:

```bash
cd benches && cargo install --path . --bin foundry-bench
```

#### Run with default settings

```bash
# Run all benchmarks on default repos with stable and nightly versions
foundry-bench --versions stable,nightly
```

#### Run with custom configurations

```bash
# Bench specific versions
foundry-bench --versions stable,nightly,v1.0.0

# Run on specific repositories. Default rev for the repo is "main"
foundry-bench --repos ithacaxyz/account,Vectorized/solady

# Test specific repository with custom revision
foundry-bench --repos ithacaxyz/account:main,Vectorized/solady:v0.0.123

# Run only specific benchmarks
foundry-bench --benchmarks forge_build_with_cache,forge_test

# Run only fuzz tests
foundry-bench --benchmarks forge_fuzz_test

# Run coverage benchmark
foundry-bench --benchmarks forge_coverage

# Run focused symbolic tests and report solver counters
foundry-bench --repos Vectorized/solady:v0.1.26,SorellaLabs/angstrom:73b55b8eca667b9a50fa4d8b6a7f45ec647420f5,farcasterxyz/contracts:3f37e21db8e9c6319b4a3d5f62b1c514ef01c36b --benchmarks forge_symbolic_test

# Combine options
foundry-bench \
  --versions stable,nightly \
  --repos ithacaxyz/account \
  --benchmarks forge_build_with_cache

# Force install Foundry versions
foundry-bench --force-install

# Verbose output to see hyperfine logs
foundry-bench --verbose

# Output to specific directory
foundry-bench --output-dir ./results --output-file LATEST_RESULTS.md
```

## Branch vs master PR-body workflow

Use this workflow when preparing performance numbers for a PR body. It keeps the
baseline and candidate builds isolated and makes the final table easy to audit.

### Command timing with foundry-bench

Use `foundry-bench` when the performance claim is about elapsed time for a
Foundry command on an existing Solidity repository. It answers questions like:

- Did this branch make `forge build` faster or slower?
- Did cached rebuilds change?
- Did `forge test` or `forge test --isolate` change?
- Did fuzz-test execution time change?
- Did `forge coverage --ir-minimum` change?
- Did focused symbolic test wall time or solver counters change?

Do not use `foundry-bench` for long-running invariant campaign quality,
throughput, corpus, showmap, or differential-coverage comparisons. Use
`foundry-scfuzzbench` for those.

Pick `BENCHMARKS` from the changed path:

| Changed path | Suggested `BENCHMARKS` |
| --- | --- |
| Compiler, artifact caching, project graph, dependency resolution | `forge_build_no_cache,forge_build_with_cache` |
| Test runner, EVM execution, cheatcodes, traces used by tests | `forge_test` |
| Isolation semantics or per-test state reset | `forge_isolate_test` |
| Fuzz runner execution overhead, fuzz fixtures, corpus replay behavior | `forge_fuzz_test` |
| Coverage instrumentation or report generation | `forge_coverage` |
| Symbolic executor or SMT solving | `forge_symbolic_test` |

Pick `REPOS` so the external project actually exercises the changed path. Use a
single focused repo first, then add more only if the result would otherwise be
unconvincing. The workflow accepts the same repo syntax as normal
`foundry-bench`: `org/repo[:rev][ <extra forge args>]`.

Examples:

```bash
# Test-runner or EVM execution change.
BENCHMARKS=forge_test
REPOS="ithacaxyz/account:v0.5.7"

# Build/cache change.
BENCHMARKS=forge_build_no_cache,forge_build_with_cache
REPOS="vectorized/solady:v0.1.26"

# Coverage change.
BENCHMARKS=forge_coverage
REPOS="uniswap/v4-core:46c6834698c48bc4a463a86d8420f4eb1d7f3b75"

# Symbolic change with focused counters.
BENCHMARKS=forge_symbolic_test
REPOS="Vectorized/solady:v0.1.26"
```

Run the matched branch-vs-base timing comparison:

```bash
BASE_REF=origin/master \
BENCHMARKS=forge_test \
REPOS="ithacaxyz/account:v0.5.7,vectorized/solady:v0.1.26" \
benches/scripts/pr-bench.sh
```

The script builds the `foundry-bench` harness from the current checkout once,
then points it at each checked-out workspace with
`FOUNDRY_BENCH_WORKSPACE_ROOT`. The `local` version builds and activates that
workspace with `FOUNDRY_BENCH_LOCAL_BUILD_PROFILE=profiling` before each run.
By default, the PR script sets `FOUNDRY_BENCH_LOCAL_BUILD_BINS=forge` so forge
benchmarks do not build unused targets. Set it to a comma- or whitespace-
separated list such as `forge,cast,anvil,chisel` when a run needs more binaries.
Keep `REPOS`, `BENCHMARKS`, and any per-repo extra arguments identical for
`master` and `candidate`. Override `CANDIDATE_REF`, `RUN_ID`, or `BENCH_ROOT`
when needed.

For PR bodies, reduce the two JSON/Markdown outputs into a concise table:

```md
### Results

| Benchmark | master | this PR | delta |
| --- | ---: | ---: | ---: |
| `forge_test/ithacaxyz-account` wall time | ... | ... | ... |
```

Include domain counters next to wall time when the benchmark produces them, for
example symbolic solver queries, reported solver time, invariant throughput, or
coverage relscore/relcov. If the delta is within noise, describe it as neutral
or inconclusive.

## Running scfuzzbench Campaigns

`foundry-scfuzzbench` runs a local scfuzzbench Foundry campaign, invokes the scfuzzbench
analysis/reporting pipeline, and copies stable artifacts into `<output-dir>/artifacts` for review by
humans or LLMs.

```bash
cargo run -p foundry-bench --bin foundry-scfuzzbench -- \
  --target-repo https://github.com/Recon-Fuzz/aave-v4-scfuzzbench.git \
  --target-ref v0.5.6-recon \
  --benchmark-type property \
  --timeout-seconds 60 \
  --workers 1 \
  --output-dir /tmp/foundry-scfuzzbench-aave \
  --foundry-bin "$(command -v forge)"
```

To compare a feature branch against `master` locally, run two matched campaigns:
one with a `forge` binary built from `master`, and one with a `forge` binary
built from the candidate branch. The runner intentionally records each campaign
separately; compare the resulting artifact bundles when drafting the PR body.

```bash
TARGET_REPO=https://github.com/Recon-Fuzz/aave-v4-scfuzzbench.git \
TARGET_REF=v0.5.6-recon \
BENCHMARK_TYPE=property \
TIMEOUT_SECONDS=3600 \
WORKERS=1 \
benches/scripts/pr-scfuzzbench.sh
```

Use the same `TARGET_REPO`, `TARGET_REF`, `BENCHMARK_TYPE`,
`TIMEOUT_SECONDS`, `WORKERS`, and `FOUNDRY_TEST_ARGS` for both runs. For
optimization campaigns, pass the same `PROPERTIES_PATH` to both runs. Summarize
the PR body from `llm_summary.md`, `REPORT.md`, `summary.csv`,
`cumulative.csv`, and the differential coverage artifacts in each output
directory. Override `BASE_REF`, `CANDIDATE_REF`, `RUN_ID`, or `BENCH_ROOT` when
needed.

By default the runner pins `https://github.com/tempoxyz/scfuzzbench.git@main`. Override that with
`--scfuzzbench-repo` and `--scfuzzbench-ref` while the scfuzzbench changes are being upstreamed.
Use `--foundry-bin` to benchmark an existing `forge`, `--foundry-ref` to build and benchmark a
Foundry ref from `--foundry-repo` (default `https://github.com/foundry-rs/foundry.git`), or neither
to use `forge` from `PATH`. The runner uses an isolated `HOME` so a selected `--foundry-bin` is not
shadowed by `~/.foundry/bin`. `--foundry-bin` must point to a file named `forge`; the runner resolves
`command -v forge` under the same campaign environment, verifies it is the selected binary, and
records the canonical path in `manifest.json`.

Optimization campaigns require `--properties-path`, which is passed to scfuzzbench as
`SCFUZZBENCH_PROPERTIES_PATH` and must be relative to the target repository. If GNU `timeout` or
GNU-style `sed -i` is unavailable, the runner installs local shims in the work directory and prepends
it to the campaign `PATH`. On platforms where `date -Is` is unavailable, it also installs a local
`date` shim for scfuzzbench log timestamps. If `local-run.sh` exits non-zero, the runner stops before
analysis so a failed setup or campaign cannot be reported as a successful artifact bundle.

The artifact bundle exposes:

- `REPORT.md`
- `events.csv`, `summary.csv`, `cumulative.csv`
- throughput/progress CSVs
- `showmap_campaign_manifest.json` and `showmap_campaigns/`
- `differential_coverage_relscores.csv`
- `differential_coverage_relcov.csv`
- runner resource and broken invariant reports
- optional `lcov-diff/` outputs when scfuzzbench produces coverage-diff files
- `llm_summary.md` and `manifest.json`, including the selected canonical `forge` path, optional
  Foundry repo/ref metadata, and optional `properties_path`

#### Command-line Options

- `--versions <VERSIONS>` - Comma-separated list of Foundry versions (default: stable,nightly)
- `--repos <REPOS>` - Comma-separated list of repos in org/repo[:rev] format (default: ithacaxyz/account:v0.3.2,Vectorized/solady:v0.1.22)
- `--benchmarks <BENCHMARKS>` - Comma-separated list of benchmarks to run
- `--force-install` - Force installation of Foundry versions
- `--verbose` - Show detailed benchmark output
- `--output-dir <DIR>` - Directory for output files (default: benches)
- `--output-file <FILE_NAME.md>` - Name of the output file (default: LATEST.md)

## Benchmark Structure

- `forge_test` - Benchmarks non-isolated `forge test` command across repos
- `forge_build_no_cache` - Benchmarks `forge build` with clean cache
- `forge_build_with_cache` - Benchmarks `forge build` with existing cache
- `forge_fuzz_test` - Benchmarks non-isolated `forge test` with only fuzz tests (tests with parameters)
- `forge_coverage` - Benchmarks `forge coverage --ir-minimum` command across repos
- `forge_isolate_test` - Benchmarks isolated `forge test` command across repos
- `forge_symbolic_test` - Benchmarks focused `forge test --symbolic --json` checks and reports symbolic solver counters

## Configuration

The benchmark binary uses command-line arguments to configure which repositories and versions to test. The default repositories are:

- `ithacaxyz/account:v0.3.2`
- `Vectorized/solady:v0.1.22`

You can override these using the `--repos` flag with the format `org/repo[:rev]`.

## Results

Benchmark results are saved to `benches/LATEST.md` (or custom output file specified with `--output-file`). The report includes:

- Summary of versions and repositories tested
- Performance comparison tables for each benchmark type showing:
  - Mean execution time
  - Min/Max times
  - Standard deviation
  - Relative performance comparison between versions
- System information (OS, CPU cores)
- Detailed hyperfine benchmark results in JSON format

## Troubleshooting

1. **Foundry version not found**: Use `--force-install` flag or manually install with `foundryup --install <version>`
2. **Repository clone fails**: Check network connectivity and repository access
3. **Build failures**: Some repositories may have specific dependencies - check their README files
4. **Hyperfine not found**: Install hyperfine using the instructions in Prerequisites
5. **npm/Node.js errors**: Ensure Node.js and npm are installed for repositories that require them
