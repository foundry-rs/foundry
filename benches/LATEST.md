# 📊 Foundry Benchmark Results

**Generated at**: 2025-07-18 23:05:00 UTC

## Forge Test

### Repositories Tested

1. [ithacaxyz/account](https://github.com/ithacaxyz/account)
2. [Vectorized/solady](https://github.com/Vectorized/solady)
3. [sparkdotfi/spark-psm](https://github.com/sparkdotfi/spark-psm)
4. [Uniswap/v4-core](https://github.com/Uniswap/v4-core)

### Foundry Versions

- **v1.2.3**: forge Version: 1.2.3-v1.2.3 (a813a2c 2025-06-08)
- **nightly-05918765cb239024e9ca396825abb9f46257419a**: forge Version: 1.2.3-nightly (0591876 2025-07-15)
- **nightly-0af43412f809c998d8b2fe69a1c9a789b7ebd085**: forge Version: 1.3.0-nightly (0af4341 2025-07-17)

| Repository           | v1.2.3  | nightly-05918765cb239024e9ca396825abb9f46257419a | nightly-0af43412f809c998d8b2fe69a1c9a789b7ebd085 |
| -------------------- | ------- | ------------------------------------------------ | ------------------------------------------------ |
| ithacaxyz-account    | 6.74 s  | 3.24 s                                           | 3.52 s                                           |
| solady               | 2.77 s  | 2.76 s                                           | 2.71 s                                           |
| sparkdotfi-spark-psm | 1m 3.7s | 1m 1.2s                                          | 1m 5.3s                                          |
| Uniswap-v4-core      | 8.04 s  | 7.58 s                                           | 8.44 s                                           |

## Forge Fuzz Test

| Repository           | v1.2.3 | nightly-05918765cb239024e9ca396825abb9f46257419a | nightly-0af43412f809c998d8b2fe69a1c9a789b7ebd085 |
| -------------------- | ------ | ------------------------------------------------ | ------------------------------------------------ |
| ithacaxyz-account    | 3.92 s | 3.37 s                                           | 3.52 s                                           |
| solady               | 2.96 s | 2.66 s                                           | 2.82 s                                           |
| sparkdotfi-spark-psm | 3.68 s | 3.52 s                                           | 3.63 s                                           |
| Uniswap-v4-core      | 8.06 s | 7.82 s                                           | 8.31 s                                           |

## Forge Build

### Repositories Tested

1. [ithacaxyz/account](https://github.com/ithacaxyz/account)
2. [Vectorized/solady](https://github.com/Vectorized/solady)
3. [sparkdotfi/spark-psm](https://github.com/sparkdotfi/spark-psm)
4. [Uniswap/v4-core](https://github.com/Uniswap/v4-core)

### Foundry Versions

- **v1.2.3**: forge Version: 1.2.3-v1.2.3 (a813a2c 2025-06-08)
- **nightly-05918765cb239024e9ca396825abb9f46257419a**: forge Version: 1.2.3-nightly (0591876 2025-07-15)
- **nightly-0af43412f809c998d8b2fe69a1c9a789b7ebd085**: forge Version: 1.3.0-nightly (0af4341 2025-07-17)

### No Cache

| Repository           | v1.2.3   | nightly-05918765cb239024e9ca396825abb9f46257419a | nightly-0af43412f809c998d8b2fe69a1c9a789b7ebd085 |
| -------------------- | -------- | ------------------------------------------------ | ------------------------------------------------ |
| ithacaxyz-account    | 9.32 s   | 9.32 s                                           | 9.46 s                                           |
| solady               | 15.01 s  | 14.97 s                                          | 14.81 s                                          |
| sparkdotfi-spark-psm | 13.42 s  | 13.29 s                                          | 13.28 s                                          |
| Uniswap-v4-core      | 2m 11.6s | 2m 7.4s                                          | 2m 6.3s                                          |

### With Cache

| Repository           | v1.2.3  | nightly-05918765cb239024e9ca396825abb9f46257419a | nightly-0af43412f809c998d8b2fe69a1c9a789b7ebd085 |
| -------------------- | ------- | ------------------------------------------------ | ------------------------------------------------ |
| ithacaxyz-account    | 0.203 s | 0.205 s                                          | 0.201 s                                          |
| solady               | 0.092 s | 0.090 s                                          | 0.094 s                                          |
| sparkdotfi-spark-psm | 0.170 s | 0.174 s                                          | 0.173 s                                          |
| Uniswap-v4-core      | 0.135 s | 0.139 s                                          | 0.135 s                                          |

## Forge Coverage

### Repositories Tested

1. [ithacaxyz/account](https://github.com/ithacaxyz/account)
2. [sparkdotfi/spark-psm](https://github.com/sparkdotfi/spark-psm)
3. [Uniswap/v4-core](https://github.com/Uniswap/v4-core)

### Foundry Versions

- **v1.2.3**: forge Version: 1.2.3-v1.2.3 (a813a2c 2025-06-08)
- **nightly-05918765cb239024e9ca396825abb9f46257419a**: forge Version: 1.2.3-nightly (0591876 2025-07-15)
- **nightly-0af43412f809c998d8b2fe69a1c9a789b7ebd085**: forge Version: 1.3.0-nightly (0af4341 2025-07-17)

| Repository           | v1.2.3   | nightly-05918765cb239024e9ca396825abb9f46257419a | nightly-0af43412f809c998d8b2fe69a1c9a789b7ebd085 |
| -------------------- | -------- | ------------------------------------------------ | ------------------------------------------------ |
| ithacaxyz-account    | 16.33 s  | 17.31 s                                          | 16.43 s                                          |
| sparkdotfi-spark-psm | 3m 52.9s | 4m 12.8s                                         | 4m 15.0s                                         |
| Uniswap-v4-core      | 1m 40.6s | 1m 42.7s                                         | 1m 47.5s                                         |

## System Information

- **OS**: macos
- **CPU**: 8
- **Rustc**: rustc 1.90.0-nightly (3014e79f9 2025-07-15)
