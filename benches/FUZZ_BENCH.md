# Foundry Benchmark Results

**Date**: 2025-07-04 11:05:43

## Summary

Benchmarked 2 Foundry versions across 2 repositories.

### Repositories Tested

1. [ithacaxyz/account](https://github.com/ithacaxyz/account)
2. [Vectorized/solady](https://github.com/Vectorized/solady)

### Foundry Versions

- **stable**: forge Version: 1.2.3-stable (a813a2c 2025-06-08)
- **nightly**: forge Version: 1.2.3-nightly (51650ea 2025-06-27)

## Forge Fuzz Test

| Repository        | stable | nightly |
| ----------------- | ------ | ------- |
| ithacaxyz-account | 3.62 s | 3.31 s  |
| solady            | 2.82 s | 2.65 s  |

## System Information

- **OS**: macos
- **CPU**: 8
- **Rustc**: rustc 1.89.0-nightly (d97326eab 2025-05-15)
