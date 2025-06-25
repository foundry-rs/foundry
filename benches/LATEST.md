# Foundry Benchmarking Results

**Generated on:** Wed Jun 25 17:43:45 IST 2025  
**Tool:** Criterion.rs with criterion-table  
**Foundry Versions Tested:** stable nightly   
**Repositories Tested:** account solady v4-core morpho-blue spark-psm   

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
    - [forge-build-with-cache](#forge-build-with-cache)

## Benchmark Results

### forge-build-with-cache

|               | `stable`                  | `nightly`                         |
|:--------------|:--------------------------|:--------------------------------- |
| **`account`** | `164.00 ms` (✅ **1.00x**) | `166.34 ms` (✅ **1.01x slower**)  |

---
Made with [criterion-table](https://github.com/nu11ptr/criterion-table)

[0;34m[INFO][0m Getting Foundry version information...
## Foundry Version Details

### stable

```
foundryup: use - forge Version: 1.2.3-stable
Commit SHA: a813a2cee7dd4926e7c56fd8a785b54f32e0d10f
Build Timestamp: 2025-06-08T15:42:50.507050000Z (1749397370)
Build Profile: maxperf
foundryup: use - cast Version: 1.2.3-stable
Commit SHA: a813a2cee7dd4926e7c56fd8a785b54f32e0d10f
Build Timestamp: 2025-06-08T15:42:50.507050000Z (1749397370)
Build Profile: maxperf
foundryup: use - anvil Version: 1.2.3-stable
Commit SHA: a813a2cee7dd4926e7c56fd8a785b54f32e0d10f
Build Timestamp: 2025-06-08T15:42:50.507050000Z (1749397370)
Build Profile: maxperf
foundryup: use - chisel Version: 1.2.3-stable
Commit SHA: a813a2cee7dd4926e7c56fd8a785b54f32e0d10f
Build Timestamp: 2025-06-08T15:42:50.507050000Z (1749397370)
Build Profile: maxperf
forge Version: 1.2.3-stable
```

### nightly

```
foundryup: use - forge Version: 1.2.3-nightly
Commit SHA: b515c90b9be9645b844943fc6d54f2304b83f75f
Build Timestamp: 2025-06-18T06:02:35.553006000Z (1750226555)
Build Profile: maxperf
foundryup: use - cast Version: 1.2.3-nightly
Commit SHA: b515c90b9be9645b844943fc6d54f2304b83f75f
Build Timestamp: 2025-06-18T06:02:35.553006000Z (1750226555)
Build Profile: maxperf
foundryup: use - anvil Version: 1.2.3-nightly
Commit SHA: b515c90b9be9645b844943fc6d54f2304b83f75f
Build Timestamp: 2025-06-18T06:02:35.553006000Z (1750226555)
Build Profile: maxperf
foundryup: use - chisel Version: 1.2.3-nightly
Commit SHA: b515c90b9be9645b844943fc6d54f2304b83f75f
Build Timestamp: 2025-06-18T06:02:35.553006000Z (1750226555)
Build Profile: maxperf
forge Version: 1.2.3-nightly
```

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
- **Date:** Wed Jun 25 17:43:46 IST 2025

## Raw Data

Detailed benchmark data and HTML reports are available in:
- `target/criterion/` - Individual benchmark reports
- `target/criterion/report/index.html` - Combined HTML report

