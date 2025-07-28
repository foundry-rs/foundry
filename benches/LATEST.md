# Foundry Benchmark Results

**Date**: 2025-07-24 14:05:02

## Repositories Tested

1. [ithacaxyz/account](https://github.com/ithacaxyz/account)
2. [Vectorized/solady](https://github.com/Vectorized/solady)
3. [Uniswap/v4-core](https://github.com/Uniswap/v4-core)
4. [sparkdotfi/spark-psm](https://github.com/sparkdotfi/spark-psm)

## Foundry Versions

- **v1.2.3**: forge Version: 1.2.3-v1.2.3 (a813a2c 2025-06-08)
- **nightly-9c3feff90b6532126b4391dfb4570401c8a6174e**: forge Version: 1.3.0-nightly (9c3feff 2025-07-24)

## Forge Test

| Repository           | v1.2.3  | nightly-9c3feff90b6532126b4391dfb4570401c8a6174e |
| -------------------- | ------- | ------------------------------------------------ |
| ithacaxyz-account    | 3.69 s  | 3.12 s                                           |
| solady               | 2.95 s  | 2.32 s                                           |
| Uniswap-v4-core      | 8.03 s  | 6.76 s                                           |
| sparkdotfi-spark-psm | 57.02 s | 44.76 s                                          |

## Forge Fuzz Test

| Repository           | v1.2.3 | nightly-9c3feff90b6532126b4391dfb4570401c8a6174e |
| -------------------- | ------ | ------------------------------------------------ |
| ithacaxyz-account    | 3.58 s | 3.39 s                                           |
| solady               | 3.34 s | 2.54 s                                           |
| Uniswap-v4-core      | 8.03 s | 7.46 s                                           |
| sparkdotfi-spark-psm | 3.70 s | 3.06 s                                           |

## Forge Test (Isolated)

| Repository           | v1.2.3  | nightly-9c3feff90b6532126b4391dfb4570401c8a6174e |
| -------------------- | ------- | ------------------------------------------------ |
| solady               | 3.02 s  | 3.18 s                                           |
| Uniswap-v4-core      | 8.98 s  | 8.34 s                                           |
| sparkdotfi-spark-psm | 58.19 s | 54.06 s                                          |

## Forge Build (No Cache)

| Repository           | v1.2.3  | nightly-9c3feff90b6532126b4391dfb4570401c8a6174e |
| -------------------- | ------- | ------------------------------------------------ |
| ithacaxyz-account    | 9.53 s  | 9.58 s                                           |
| solady               | 15.29 s | 14.97 s                                          |
| Uniswap-v4-core      | 2m 8.3s | 2m 9.0s                                          |
| sparkdotfi-spark-psm | 13.33 s | 13.29 s                                          |

## Forge Build (With Cache)

| Repository           | v1.2.3  | nightly-9c3feff90b6532126b4391dfb4570401c8a6174e |
| -------------------- | ------- | ------------------------------------------------ |
| ithacaxyz-account    | 0.199 s | 0.206 s                                          |
| solady               | 0.092 s | 0.089 s                                          |
| Uniswap-v4-core      | 0.190 s | 0.136 s                                          |
| sparkdotfi-spark-psm | 0.239 s | 0.202 s                                          |

## Forge Coverage

| Repository           | v1.2.3   | nightly-9c3feff90b6532126b4391dfb4570401c8a6174e |
| -------------------- | -------- | ------------------------------------------------ |
| ithacaxyz-account    | 16.51 s  | 16.62 s                                          |
| Uniswap-v4-core      | 1m 40.4s | 1m 40.9s                                         |
| sparkdotfi-spark-psm | 3m 55.3s | 4m 0.4s                                          |

## System Information

- **OS**: macos
- **CPU**: 8
- **Rustc**: rustc 1.90.0-nightly (3014e79f9 2025-07-15)
