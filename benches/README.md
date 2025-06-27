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

5. **Benchmark tools** - Required for generating reports
   ```bash
   cargo install cargo-criterion
   cargo install criterion-table
   ```

## Running Benchmarks

### Run the complete benchmark suite

```bash
cargo run
```

This will:

1. Check and install required Foundry versions
2. Run all benchmark suites (forge_test, forge_build_no_cache, forge_build_with_cache)
3. Generate comparison tables using criterion-table
4. Create the final LATEST.md report

### Run individual benchmark suites

```bash
./run_benchmarks.sh
```

### Run specific benchmark

```bash
cargo criterion --bench forge_test
cargo criterion --bench forge_build_no_cache
cargo criterion --bench forge_build_with_cache
```

## Benchmark Structure

- `forge_test` - Benchmarks `forge test` command across repos
- `forge_build_no_cache` - Benchmarks `forge build` with clean cache
- `forge_build_with_cache` - Benchmarks `forge build` with existing cache

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

Benchmark results are displayed in the terminal and saved as HTML reports. The reports show:

- Execution time statistics (mean, median, standard deviation)
- Comparison between different Foundry versions
- Performance trends across repositories

## Troubleshooting

1. **Foundry version not found**: Ensure the version is installed with `foundryup --install <version>`
2. **Repository clone fails**: Check network connectivity and repository access
3. **Build failures**: Some repositories may have specific dependencies - check their README files
