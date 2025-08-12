# Foundry Benchmark Results

**Date**: 2025-07-31 20:38:35

## Repositories Tested

1. [ithacaxyz/account](https://github.com/ithacaxyz/account)
2. [Vectorized/solady](https://github.com/Vectorized/solady)
3. [Uniswap/v4-core](https://github.com/Uniswap/v4-core)
4. [sparkdotfi/spark-psm](https://github.com/sparkdotfi/spark-psm)

## Foundry Versions

- **v1.2.3**: forge Version: 1.2.3-v1.2.3 (a813a2c 2025-06-08)
- **v1.3.0**: forge 1.3.0-v1.3.0 (b918f9b 2025-07-31)

## Forge Test

| Repository           | v1.2.3  | v1.3.0  |
| -------------------- | ------- | ------- |
| ithacaxyz-account    | 3.53 s  | 3.15 s  |
| solady               | 2.72 s  | 2.30 s  |
| Uniswap-v4-core      | 8.14 s  | 7.14 s  |
| sparkdotfi-spark-psm | 57.18 s | 47.27 s |

## Forge Fuzz Test

| Repository           | v1.2.3 | v1.3.0 |
| -------------------- | ------ | ------ |
| ithacaxyz-account    | 3.88 s | 3.05 s |
| solady               | 2.93 s | 2.46 s |
| Uniswap-v4-core      | 8.20 s | 6.90 s |
| sparkdotfi-spark-psm | 3.81 s | 2.99 s |

## Forge Test (Isolated)

| Repository           | v1.2.3  | v1.3.0  |
| -------------------- | ------- | ------- |
| solady               | 2.98 s  | 2.57 s  |
| Uniswap-v4-core      | 8.61 s  | 7.72 s  |
| sparkdotfi-spark-psm | 53.01 s | 45.94 s |

## Forge Build (No Cache)

| Repository           | v1.2.3  | v1.3.0  |
| -------------------- | ------- | ------- |
| ithacaxyz-account    | 9.25 s  | 9.27 s  |
| solady               | 14.59 s | 14.65 s |
| Uniswap-v4-core      | 2m 3.5s | 2m 4.0s |
| sparkdotfi-spark-psm | 13.24 s | 13.21 s |

## Forge Build (With Cache)

| Repository           | v1.2.3  | v1.3.0  |
| -------------------- | ------- | ------- |
| ithacaxyz-account    | 0.195 s | 0.198 s |
| solady               | 0.086 s | 0.089 s |
| Uniswap-v4-core      | 0.131 s | 0.132 s |
| sparkdotfi-spark-psm | 0.181 s | 0.171 s |

## Forge Coverage

| Repository           | v1.2.3   | v1.3.0   |
| -------------------- | -------- | -------- |
| ithacaxyz-account    | 15.79 s  | 15.63 s  |
| Uniswap-v4-core      | 1m 36.6s | 1m 35.2s |
| sparkdotfi-spark-psm | 3m 38.4s | 3m 50.0s |

## System Information

- **OS**: macos
- **CPU**: 8
- **Rustc**: rustc 1.90.0-nightly (3014e79f9 2025-07-15)
