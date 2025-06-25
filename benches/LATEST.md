# Foundry Benchmarking Results

**Generated on:** Wed Jun 25 18:27:00 IST 2025  
**Foundry Versions Tested:** stable nightly

## Repositories Tested

1. [ithacaxyz-account](https://github.com/ithacaxyz/main)
2. [solady](https://github.com/Vectorized/main)
3. [v4-core](https://github.com/Uniswap/main)
4. [morpho-blue](https://github.com/morpho-org/main)
5. [spark-psm](https://github.com/marsfoundation/master)

## Summary

This report contains comprehensive benchmarking results comparing different Foundry versions across multiple projects using Criterion.rs for precise performance measurements.

The following benchmarks were performed:

1. **forge-test** - Running the test suite (10 samples each)
2. **forge-build-no-cache** - Clean build without cache (10 samples each)
3. **forge-build-with-cache** - Build with warm cache (10 samples each)

---

## Performance Comparison Tables

# Benchmarks

## Table of Contents

- [Benchmark Results](#benchmark-results)
  - [forge-test](#forge-test)
  - [forge-build-no-cache](#forge-build-no-cache)
  - [forge-build-with-cache](#forge-build-with-cache)

## Benchmark Results

### forge-test

|                         | `stable`                | `nightly`                      |
| :---------------------- | :---------------------- | :----------------------------- |
| **`ithacaxyz-account`** | `3.73 s` (✅ **1.00x**) | `3.30 s` (✅ **1.13x faster**) |

### forge-build-no-cache

|                         | `stable`                 | `nightly`                       |
| :---------------------- | :----------------------- | :------------------------------ |
| **`ithacaxyz-account`** | `14.32 s` (✅ **1.00x**) | `14.37 s` (✅ **1.00x slower**) |

### forge-build-with-cache

|                         | `stable`                   | `nightly`                         |
| :---------------------- | :------------------------- | :-------------------------------- |
| **`ithacaxyz-account`** | `162.64 ms` (✅ **1.00x**) | `167.49 ms` (✅ **1.03x slower**) |

---

Made with [criterion-table](https://github.com/nu11ptr/criterion-table)

## System Information

- **OS:** Darwin
- **Architecture:** arm64
- **Date:** Wed Jun 25 18:27:01 IST 2025

## Raw Data

Detailed benchmark data and HTML reports are available in:

- `target/criterion/` - Individual benchmark reports
