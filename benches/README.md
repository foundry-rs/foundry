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

By default the runner pins `https://github.com/tempoxyz/scfuzzbench.git@main`. Override that with
`--scfuzzbench-repo` and `--scfuzzbench-ref` while the scfuzzbench changes are being upstreamed.
Use `--foundry-bin` to benchmark an existing `forge`, `--foundry-ref` to build and benchmark a
Foundry ref from `--foundry-repo` (default `https://github.com/foundry-rs/foundry.git`), or neither
to use `forge` from `PATH`. The runner uses an isolated `HOME` so a selected `--foundry-bin` is not
shadowed by `~/.foundry/bin`. `--foundry-bin` must point to a file named `forge`; the runner resolves
`command -v forge` under the same campaign environment, verifies it is the selected binary, and
records the canonical path in `manifest.json`.

Optimization campaigns require `--properties-path`, which is passed to scfuzzbench as
`SCFUZZBENCH_PROPERTIES_PATH` and must be relative to the target repository. If GNU `timeout` is
unavailable, the runner installs a local `timeout` shim in the work directory and prepends it to the
campaign `PATH`. On platforms where `date -Is` is unavailable, it also installs a local `date` shim
for scfuzzbench log timestamps. If `local-run.sh` exits non-zero, the runner stops before analysis so
a failed setup or campaign cannot be reported as a successful artifact bundle.

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

- `forge_test` - Benchmarks `forge test` command across repos
- `forge_build_no_cache` - Benchmarks `forge build` with clean cache
- `forge_build_with_cache` - Benchmarks `forge build` with existing cache
- `forge_fuzz_test` - Benchmarks `forge test` with only fuzz tests (tests with parameters)
- `forge_coverage` - Benchmarks `forge coverage --ir-minimum` command across repos

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
