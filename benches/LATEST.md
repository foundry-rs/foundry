# Forge Benchmarking Results

**Generated on:** Thu 12 Jun 2025 16:57:20 CEST
**Hyperfine Version:** hyperfine 1.19.0
**Foundry Versions Tested:** stable nightly-ac0411d0e3b9632247c9aea9535472eda09a57ae nightly
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

| Project               | stable (s) | nightly-ac0411d0e3b9632247c9aea9535472eda09a57ae (s) | nightly (s) |
| --------------------- | ---------: | ---------------------------------------------------: | ----------: |
| **ithacaxyz-account** |      4.662 |                                                3.738 |       5.588 |
| **solady**            |      3.559 |                                                2.933 |       3.517 |

### forge build no cache

Mean execution time in seconds (lower is better):

| Project               | stable (s) | nightly-ac0411d0e3b9632247c9aea9535472eda09a57ae (s) | nightly (s) |
| --------------------- | ---------: | ---------------------------------------------------: | ----------: |
| **ithacaxyz-account** |     10.777 |                                               10.982 |      10.979 |
| **solady**            |     17.486 |                                               17.139 |      17.509 |

### forge build with cache

Mean execution time in seconds (lower is better):

| Project               | stable (s) | nightly-ac0411d0e3b9632247c9aea9535472eda09a57ae (s) | nightly (s) |
| --------------------- | ---------: | ---------------------------------------------------: | ----------: |
| **ithacaxyz-account** |      0.111 |                                                0.113 |       0.158 |
| **solady**            |      0.084 |                                                0.089 |       0.108 |

## Foundry Version Details

### stable

```
forge Version: 1.2.3-stable
```

### nightly-ac0411d0e3b9632247c9aea9535472eda09a57ae

```
forge Version: 1.2.3-nightly
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
- N/A indicates benchmark failed.

## System Information

- **OS:** Darwin
- **Architecture:** arm64
- **Date:** Thu 12 Jun 2025 16:57:21 CEST

## Raw Data

Raw JSON benchmark data is available in: `/Users/yash/dev/paradigm/foundry-rs/foundry/benches/benchmark_results/json_20250612_165120`
