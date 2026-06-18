# Foundry Benchmark Results

**Date**: 2026-04-24 23:10:24

## Repositories Tested

1. [ithacaxyz/account](https://github.com/ithacaxyz/account)
2. [Vectorized/solady](https://github.com/Vectorized/solady)
3. [Uniswap/v4-core](https://github.com/Uniswap/v4-core)
4. [sparkdotfi/spark-psm](https://github.com/sparkdotfi/spark-psm)
5. [aave/aave-v4](https://github.com/aave/aave-v4)

## Foundry Versions

- **v1.5.1**: forge Version: 1.5.1-v1.5.1 (b0a9dd9 2025-12-19)
- **nightly**: forge Version: 1.6.0-nightly (a249f5c 2026-04-24)

## Forge Test

| Repository           | v1.5.1   | nightly  |
| -------------------- | -------- | -------- |
| vectorized-solady    | 1.46 s   | 1.38 s   |
| aave-aave-v4         | 4m 14.2s | 3m 29.1s |

## Forge Fuzz Test

| Repository           | v1.5.1    | nightly  |
| -------------------- | --------- | -------- |
| ithacaxyz-account    | 2.81 s    | 1.59 s   |
| vectorized-solady    | 1.40 s    | 1.34 s   |
| Uniswap-v4-core      | 3.01 s    | 2.87 s   |
| sparkdotfi-spark-psm | 2.04 s    | 1.87 s   |
| aave-aave-v4         | 3m 46.0s  | 3m 17.3s |

## Forge Test (Isolated)

| Repository           | v1.5.1   | nightly  |
| -------------------- | -------- | -------- |
| Uniswap-v4-core      | 3.50 s   | 3.48 s.  |
| aave-aave-v4         | 4m 14.0s | 3m 53.4s |

## Forge Build (No Cache)

| Repository           | v1.5.1   | nightly  |
| -------------------- | -------- | -------- |
| ithacaxyz-account    | 26.06 s  | 26.61 s  |
| vectorized-solady    | 14.20 s  | 14.26 s  |
| Uniswap-v4-core      | 2m 1.3s  | 2m 5.0s  |
| sparkdotfi-spark-psm | 15.16 s  | 15.30 s  |
| aave-aave-v4         | 3m 37.0s | 3m 35.1s |

## Forge Build (With Cache)

| Repository           | v1.5.1  | nightly |
| -------------------- | ------- | ------- |
| ithacaxyz-account    | 0.167 s | 0.201 s |
| vectorized-solady    | 0.099 s | 0.098 s |
| Uniswap-v4-core      | 0.139 s | 0.140 s |
| sparkdotfi-spark-psm | 0.168 s | 0.173 s |
| aave-aave-v4         | 0.370 s | 0.357 s |

## Forge Coverage

| Repository           | v1.5.1    | nightly    |
| -------------------- | --------- | ---------- |
| Uniswap-v4-core      | 1m 13.9s  | 1m 10.3s   |
| sparkdotfi-spark-psm | 2m 54.7s  | 2m 50.0s   |
| aave-aave-v4         | 11m 20.8s | 10m 58.7s  |

## System Information

- **OS**: macos
- **CPU**: 12
- **Rustc**: rustc 1.95.0 (59807616e 2026-04-14)