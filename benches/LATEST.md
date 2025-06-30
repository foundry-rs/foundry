# Foundry Benchmark Results

**Date**: 2025-06-30 17:23:42

## Summary

Benchmarked 2 Foundry versions across 2 repositories.

### Repositories Tested

1. [ithacaxyz/account](https://github.com/ithacaxyz/account)
2. [Vectorized/solady](https://github.com/Vectorized/solady)

### Foundry Versions

- stable
- nightly

## Forge Build Performance (With Cache)

| Repository        | stable    | nightly   |
| ----------------- | --------- | --------- |
| ithacaxyz-account | 227.20 ms | 263.73 ms |
| solady            | 148.77 ms | 192.25 ms |

## Forge Test Performance

| Repository        | stable | nightly |
| ----------------- | ------ | ------- |
| ithacaxyz-account | 4.88 s | 4.37 s  |
| solady            | 3.45 s | 3.43 s  |

## Forge Build Performance (No Cache)

| Repository        | stable  | nightly |
| ----------------- | ------- | ------- |
| ithacaxyz-account | 16.35 s | 13.85 s |
| solady            | 15.27 s | 15.12 s |

## System Information

- **OS**: macos
- **CPU**: 8
- **Rustc**: rustc 1.89.0-nightly (d97326eab 2025-05-15)
