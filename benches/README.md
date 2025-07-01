# Foundry Benchmarks

This directory contains performance benchmarks for Foundry commands across multiple repositories and Foundry versions.

## Prerequisites

Before running the benchmarks, ensure you have the following installed:

1. **Rust and Cargo** - Required for building and running the benchmarks

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **Foundryup** - The Foundry toolchain installer

   ```bash
   curl -L https://foundry.paradigm.xyz | bash
   foundryup
   ```

3. **Git** - For cloning benchmark repositories

4. **npm** - Some repositories require npm dependencies

   ```bash
   # Install Node.js and npm from https://nodejs.org/
   ```

## Running Benchmarks

### Using the Benchmark Binary

Build the benchmark runner:

```bash
cargo build --release --bin foundry-bench
```

#### Run with default settings

```bash
# Run all benchmarks on default repos with stable and nightly versions
cargo run --release --bin foundry-bench -- --versions stable,nightly
```

#### Run with custom configurations

```bash
# Bench specific versions
cargo run --release --bin foundry-bench -- --versions stable,nightly,v1.0.0

# Run on specific repositories. Default rev for the repo is "main"
cargo run --release --bin foundry-bench -- --repos ithacaxyz/account,Vectorized/solady

# Test specific repository with custom revision
cargo run --release --bin foundry-bench -- --repos ithacaxyz/account:main,Vectorized/solady:v0.0.123

# Run only specific benchmarks
cargo run --release --bin foundry-bench -- --benchmarks forge_build_with_cache,forge_test

# Run only fuzz tests
cargo run --release --bin foundry-bench -- --benchmarks forge_fuzz_test

# Run coverage benchmark
cargo run --release --bin foundry-bench -- --benchmarks forge_coverage

# Combine options
cargo run --release --bin foundry-bench -- \
  --versions stable,nightly \
  --repos ithacaxyz/account \
  --benchmarks forge_build_with_cache

# Force install Foundry versions
cargo run --release --bin foundry-bench -- --force-install

# Verbose output to see criterion logs
cargo run --release --bin foundry-bench --  --verbose

# Output to specific directory
cargo run --release --bin foundry-bench -- --output-dir ./results --output-file LATEST_RESULTS.md
```

#### Command-line Options

- `--versions <VERSIONS>` - Comma-separated list of Foundry versions (default: stable,nightly)
- `--repos <REPOS>` - Comma-separated list of repos in org/repo[:rev] format
- `--benchmarks <BENCHMARKS>` - Comma-separated list of benchmarks to run
- `--force-install` - Force installation of Foundry versions
- `--verbose` - Show detailed benchmark output
- `--output-dir <DIR>` - Directory for output files (default: current directory)
- `--output-file <FILE_NAME.md>` - Name of the output file (default: LATEST.md)

### Run individual Criterion benchmarks

```bash
# Run specific benchmark with Criterion
cargo bench --bench forge_test
cargo bench --bench forge_build_no_cache
cargo bench --bench forge_build_with_cache
cargo bench --bench forge_fuzz_test
cargo bench --bench forge_coverage
```

## Benchmark Structure

- `forge_test` - Benchmarks `forge test` command across repos
- `forge_build_no_cache` - Benchmarks `forge build` with clean cache
- `forge_build_with_cache` - Benchmarks `forge build` with existing cache
- `forge_fuzz_test` - Benchmarks `forge test` with only fuzz tests (tests with parameters)
- `forge_coverage` - Benchmarks `forge coverage --ir-minimum` command across repos

## Configuration

### Repositories

Edit `src/lib.rs` to modify the list of repositories to benchmark:

```rust
pub static BENCHMARK_REPOS: &[RepoConfig] = &[
    RepoConfig { name: "account", org: "ithacaxyz", repo: "account", rev: "main" },
    // Add more repositories here
];
```

### Foundry Versions

Edit `src/lib.rs` to modify the list of Foundry versions:

```rust
pub static FOUNDRY_VERSIONS: &[&str] = &["stable", "nightly"];
```

## Results

Benchmark results are saved to `LATEST.md` (or custom output directory). The report includes:

- Summary of versions and repositories tested
- Performance comparison tables for each benchmark type
- Execution time statistics for each repository/version combination
- System information (OS, CPU, Rust version)

Results are also stored in Criterion's format in `target/criterion/` for detailed analysis.

## GitHub Actions Integration

The benchmarks can be run automatically via GitHub Actions:

1. Go to the [Actions tab](../../actions/workflows/benchmarks.yml)
2. Click on "Foundry Benchmarks" workflow
3. Click "Run workflow"
4. Configure options:
   - PR number (optional) - Add benchmark results as a comment to a PR
   - Versions - Foundry versions to test (default: stable,nightly)
   - Repos - Custom repositories to benchmark
   - Benchmarks - Specific benchmarks to run

The workflow will:

- Build and run the benchmark binary
- Commit results to `benches/LATEST.md`
- Optionally comment on a PR with the results

## Troubleshooting

1. **Foundry version not found**: Ensure the version is installed with `foundryup --install <version>`
2. **Repository clone fails**: Check network connectivity and repository access
3. **Build failures**: Some repositories may have specific dependencies - check their README files
