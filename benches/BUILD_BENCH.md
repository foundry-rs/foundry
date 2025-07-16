# Foundry Benchmark Results

**Date**: 2025-07-16 15:05:32

## Summary

Benchmarked 3 Foundry versions across 4 repositories.

### Repositories Tested

1. [ithacaxyz/account](https://github.com/ithacaxyz/account)
2. [Vectorized/solady](https://github.com/Vectorized/solady)
3. [Uniswap/v4-core](https://github.com/Uniswap/v4-core)
4. [sparkdotfi/spark-psm](https://github.com/sparkdotfi/spark-psm)

### Foundry Versions

- **v1.2.3**: forge Version: 1.2.3-v1.2.3 (a813a2c 2025-06-08)
- **nightly-05918765cb239024e9ca396825abb9f46257419a**: forge Version: 1.2.3-nightly (0591876 2025-07-15)
- **nightly-13c4502c80ceae8429056eefc1e6a3b1e4e86b53**: forge Version: 1.3.0-nightly (13c4502 2025-07-16)

## Forge Build (With Cache)

| Repository           | v1.2.3  | nightly-05918765cb239024e9ca396825abb9f46257419a | nightly-13c4502c80ceae8429056eefc1e6a3b1e4e86b53 |
| -------------------- | ------- | ------------------------------------------------ | ------------------------------------------------ |
| ithacaxyz-account    | 4.94 s  | 5.79 s                                           | 4.33 s                                           |
| solady               | 6.54 s  | 5.42 s                                           | 6.21 s                                           |
| Uniswap-v4-core      | 43.85 s | 47.91 s                                          | 45.56 s                                          |
| sparkdotfi-spark-psm | 6.65 s  | 6.85 s                                           | 6.74 s                                           |

## Forge Build (No Cache)

| Repository           | v1.2.3  | nightly-05918765cb239024e9ca396825abb9f46257419a | nightly-13c4502c80ceae8429056eefc1e6a3b1e4e86b53 |
| -------------------- | ------- | ------------------------------------------------ | ------------------------------------------------ |
| ithacaxyz-account    | 4.84 s  | 5.78 s                                           | 4.00 s                                           |
| solady               | 6.52 s  | 5.43 s                                           | 6.09 s                                           |
| Uniswap-v4-core      | 46.92 s | 48.00 s                                          | 45.68 s                                          |
| sparkdotfi-spark-psm | 5.91 s  | 6.29 s                                           | 6.35 s                                           |

## System Information

- **OS**: macos
- **CPU**: 8
- **Rustc**: rustc 1.90.0-nightly (3014e79f9 2025-07-15)
