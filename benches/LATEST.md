# Forge Benchmarking Results

**Generated on:** Wed 18 Jun 2025 17:46:19 BST
**Hyperfine Version:** hyperfine 1.19.0
**Foundry Versions Tested:** stable nightly
**Repositories Tested:** ithacaxyz-account solady

## Summary

This report contains comprehensive benchmarking results comparing different Foundry versions across multiple projects.
The following benchmarks were performed:

1. **forge test - Running the test suite (5 runs, 1 warmup)**
2. **forge build (no cache) - Clean build without cache (5 runs, cache cleaned after each run)**
3. **forge build (with cache) - Build with warm cache (5 runs, 1 warmup)**

---

## Performance Comparison Tables

### forge test

Mean execution time in seconds (lower is better):

| Project | stable (s) | nightly (s) |
|------|--------:|--------:|
| **ithacaxyz-account** | 5.791 | 3.875 |
| **solady** | 3.578 | 2.966 |


### forge build no cache

Mean execution time in seconds (lower is better):

| Project | stable (s) | nightly (s) |
|------|--------:|--------:|
| **ithacaxyz-account** | 19.079 | 16.177 |
| **solady** | 27.408 | 22.745 |


### forge build with cache

Mean execution time in seconds (lower is better):

| Project | stable (s) | nightly (s) |
|------|--------:|--------:|
| **ithacaxyz-account** | 0.181 | 0.158 |
| **solady** | 0.091 | 0.103 |


## Foundry Version Details

### stable

```
forge Version: 1.2.3-stable
```

### nightly

```
forge Version: 1.2.3-nightly
```


## Notes

- All benchmarks were run with hyperfine in parallel mode
- **forge test - Running the test suite (5 runs, 1 warmup)**
- **forge build (no cache) - Clean build without cache (5 runs, cache cleaned after each run)**
- **forge build (with cache) - Build with warm cache (5 runs, 1 warmup)**
- Results show mean execution time in seconds
- N/A indicates benchmark failed or data unavailable

## System Information

- **OS:** Darwin
- **Architecture:** arm64
- **Date:** Wed 18 Jun 2025 17:46:19 BST

## Raw Data

Raw JSON benchmark data is available in: `/Users/yash/dev/paradigm/foundry-rs/foundry/benches/benchmark_results/json_20250618_174101`

