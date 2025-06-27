# Foundry Benchmarking Results

**Generated on:** Fri 27 Jun 2025 15:51:19 IST  
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

# Benchmarks

## Table of Contents

- [Benchmark Results](#benchmark-results)
    - [forge-test](#forge-test)
    - [forge-build-no-cache](#forge-build-no-cache)
    - [forge-build-with-cache](#forge-build-with-cache)

## Benchmark Results

### forge-test

|                         | `stable`               | `nightly`                      |
|:------------------------|:-----------------------|:------------------------------ |
| **`ithacaxyz-account`** | `3.75 s` (✅ **1.00x**) | `3.27 s` (✅ **1.15x faster**)  |

### forge-build-no-cache

|                         | `stable`                | `nightly`                       |
|:------------------------|:------------------------|:------------------------------- |
| **`ithacaxyz-account`** | `14.23 s` (✅ **1.00x**) | `14.25 s` (✅ **1.00x slower**)  |

### forge-build-with-cache

|                         | `stable`                  | `nightly`                         |
|:------------------------|:--------------------------|:--------------------------------- |
| **`ithacaxyz-account`** | `163.53 ms` (✅ **1.00x**) | `168.00 ms` (✅ **1.03x slower**)  |

---
Made with [criterion-table](https://github.com/nu11ptr/criterion-table)

## Notes

- All benchmarks use Criterion.rs for statistical analysis
- Each benchmark runs 10 samples by default
- Results show mean execution time with confidence intervals
- Repositories are cloned once and reused across all Foundry versions
- Build and setup operations are parallelized using Rayon
- The first version tested becomes the baseline for comparisons

## System Information

- **OS:** Darwin
- **Architecture:** arm64
- **Date:** Fri 27 Jun 2025 15:51:19 IST

## Raw Data

Detailed benchmark data and HTML reports are available in:
- `target/criterion/` - Individual benchmark reports

