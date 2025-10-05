# Foundry Benchmark Results

**Date**: 2025-10-02 12:14:23

## Repositories Tested

1. [ithacaxyz/account](https://github.com/ithacaxyz/account)
2. [Vectorized/solady](https://github.com/Vectorized/solady)
3. [Uniswap/v4-core](https://github.com/Uniswap/v4-core)
4. [sparkdotfi/spark-psm](https://github.com/sparkdotfi/spark-psm)

## Foundry Versions

- **v1.3.6**: forge Version: 1.3.6-v1.3.6 (d241588 2025-09-16)
- **v1.4.0-rc1**: forge Version: 1.4.0-v1.4.0-rc1 (bd0e4a7 2025-10-01)

## Forge Test

| Repository           | v1.3.6  | v1.4.0-rc1 |
| -------------------- | ------- | ---------- |
| ithacaxyz-account    | 3.17 s  | 2.94 s     |
| solady               | 2.28 s  | 2.10 s     |
| Uniswap-v4-core      | 7.27 s  | 6.13 s     |
| sparkdotfi-spark-psm | 43.04 s | 44.08 s    |

## Forge Fuzz Test

| Repository           | v1.3.6 | v1.4.0-rc1 |
| -------------------- | ------ | ---------- |
| ithacaxyz-account    | 3.18 s | 3.02 s     |
| solady               | 2.39 s | 2.24 s     |
| Uniswap-v4-core      | 6.84 s | 6.20 s     |
| sparkdotfi-spark-psm | 3.07 s | 2.72 s     |

## Forge Test (Isolated)

| Repository           | v1.3.6  | v1.4.0-rc1 |
| -------------------- | ------- | ---------- |
| solady               | 2.26 s  | 2.41 s     |
| Uniswap-v4-core      | 7.22 s  | 7.71 s     |
| sparkdotfi-spark-psm | 45.53 s | 50.49 s    |

## Forge Build (No Cache)

| Repository           | v1.3.6  | v1.4.0-rc1 |
| -------------------- | ------- | ---------- |
| ithacaxyz-account    | 9.16 s  | 9.08 s     |
| solady               | 14.62 s | 14.69 s    |
| Uniswap-v4-core      | 2m 3.8s | 2m 5.3s    |
| sparkdotfi-spark-psm | 13.17 s | 13.14 s    |

## Forge Build (With Cache)

| Repository           | v1.3.6  | v1.4.0-rc1 |
| -------------------- | ------- | ---------- |
| ithacaxyz-account    | 0.156 s | 0.113 s    |
| solady               | 0.089 s | 0.094 s    |
| Uniswap-v4-core      | 0.133 s | 0.127 s    |
| sparkdotfi-spark-psm | 0.173 s | 0.131 s    |

## Forge Coverage

| Repository           | v1.3.6   | v1.4.0-rc1 |
| -------------------- | -------- | ---------- |
| ithacaxyz-account    | 14.91 s  | 13.34 s    |
| Uniswap-v4-core      | 1m 34.8s | 1m 30.3s   |
| sparkdotfi-spark-psm | 3m 49.3s | 3m 40.2s   |

## System Information

- **OS**: macos
- **CPU**: 8
- **Rustc**: rustc 1.90.0-nightly (3014e79f9 2025-07-15)
