# Foundry Benchmark Results

**Date**: 2025-07-04 11:14:10

## Summary

Benchmarked 2 Foundry versions across 2 repositories.

### Repositories Tested

1. [ithacaxyz/account](https://github.com/ithacaxyz/account)
2. [Vectorized/solady](https://github.com/Vectorized/solady)

### Foundry Versions

- **stable**: forge Version: 1.2.3-stable (a813a2c 2025-06-08)
- **nightly**: forge Version: 1.2.3-nightly (51650ea 2025-06-27)

## Forge Test

| Repository        | stable | nightly |
| ----------------- | ------ | ------- |
| ithacaxyz-account | 4.07 s | 4.39 s  |
| solady            | 3.93 s | 5.26 s  |

## Forge Build (With Cache)

| Repository        | stable | nightly |
| ----------------- | ------ | ------- |
| ithacaxyz-account | 2.02 s | 4.96 s  |
| solady            | 3.22 s | 3.66 s  |

## Forge Build (No Cache)

| Repository        | stable | nightly |
| ----------------- | ------ | ------- |
| ithacaxyz-account | 3.54 s | 3.18 s  |
| solady            | 5.36 s | 3.71 s  |

## System Information

- **OS**: macos
- **CPU**: 8
- **Rustc**: rustc 1.89.0-nightly (d97326eab 2025-05-15)
