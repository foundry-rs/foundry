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

### Run all benchmarks

```bash
cargo bench
```

### Run specific benchmark

```bash
cargo bench forge_test
cargo bench forge_build_no_cache
cargo bench forge_build_with_cache
```

### Generate HTML reports

Criterion automatically generates HTML reports in `target/criterion/`. Open the reports in a browser:

```bash
open target/criterion/report/index.html
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
