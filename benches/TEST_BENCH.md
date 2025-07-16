# Foundry Benchmark Results

**Date**: 2025-07-16 14:49:56

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

## Forge Test

| Repository           | v1.2.3   | nightly-05918765cb239024e9ca396825abb9f46257419a | nightly-13c4502c80ceae8429056eefc1e6a3b1e4e86b53 |
| -------------------- | -------- | ------------------------------------------------ | ------------------------------------------------ |
| ithacaxyz-account    | 11.73 s  | 3.32 s                                           | 16.04 s                                          |
| solady               | 3.09 s   | 4.93 s                                           | 5.85 s                                           |
| Uniswap-v4-core      | 18.39 s  | 31.66 s                                          | 34.89 s                                          |
| sparkdotfi-spark-psm | 1m 39.9s | 1m 38.2s                                         | 2m 22.4s                                         |

## Forge Fuzz Test

| Repository           | v1.2.3  | nightly-05918765cb239024e9ca396825abb9f46257419a | nightly-13c4502c80ceae8429056eefc1e6a3b1e4e86b53 |
| -------------------- | ------- | ------------------------------------------------ | ------------------------------------------------ |
| ithacaxyz-account    | 13.19 s | 7.12 s                                           | 15.97 s                                          |
| solady               | 4.01 s  | 4.35 s                                           | 5.91 s                                           |
| Uniswap-v4-core      | 20.26 s | 32.36 s                                          | 27.33 s                                          |
| sparkdotfi-spark-psm | 9.22 s  | 7.39 s                                           | 15.44 s                                          |

## System Information

- **OS**: macos
- **CPU**: 8
- **Rustc**: rustc 1.90.0-nightly (3014e79f9 2025-07-15)
