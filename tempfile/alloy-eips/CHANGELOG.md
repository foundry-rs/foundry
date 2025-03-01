# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.11.1](https://github.com/alloy-rs/alloy/releases/tag/v0.11.1) - 2025-02-12

### Features

- Add helpers for the blob gas ([#2009](https://github.com/alloy-rs/alloy/issues/2009))

### Miscellaneous Tasks

- Re-export kzgsettings ([#2034](https://github.com/alloy-rs/alloy/issues/2034))
- Camelcase serde ([#2018](https://github.com/alloy-rs/alloy/issues/2018))
- Add serde support for Eip1559Estimation ([#2012](https://github.com/alloy-rs/alloy/issues/2012))

### Other

- Increase default gas limit from 30M to 36M ([#1785](https://github.com/alloy-rs/alloy/issues/1785))

## [0.11.0](https://github.com/alloy-rs/alloy/releases/tag/v0.11.0) - 2025-01-31

### Documentation

- Enable some useful rustdoc features on docs.rs ([#1890](https://github.com/alloy-rs/alloy/issues/1890))

### Features

- Unify `BlobParams` and `BlobScheduleItem` ([#1919](https://github.com/alloy-rs/alloy/issues/1919))
- Reexport eip2124 ([#1900](https://github.com/alloy-rs/alloy/issues/1900))
- Add match_versioned_hashes ([#1882](https://github.com/alloy-rs/alloy/issues/1882))

### Miscellaneous Tasks

- Release 0.11.0
- Update system contract addresses for devnet 6 ([#1975](https://github.com/alloy-rs/alloy/issues/1975))
- Feature gate serde ([#1967](https://github.com/alloy-rs/alloy/issues/1967))
- Forward arbitrary feature ([#1941](https://github.com/alloy-rs/alloy/issues/1941))
- [eips] Add super trait `Typed2718` to `Encodable2718` ([#1913](https://github.com/alloy-rs/alloy/issues/1913))
- Release 0.10.0
- Improve FromStr for `BlockNumberOrTag` to be case-insensitive ([#1891](https://github.com/alloy-rs/alloy/issues/1891))
- Shift std::error impls to core ([#1888](https://github.com/alloy-rs/alloy/issues/1888))
- Use core::error for blob validation error ([#1887](https://github.com/alloy-rs/alloy/issues/1887))
- Use safe get api  ([#1886](https://github.com/alloy-rs/alloy/issues/1886))

### Other

- Add zepter and propagate features ([#1951](https://github.com/alloy-rs/alloy/issues/1951))

### Testing

- Require serde features for tests ([#1924](https://github.com/alloy-rs/alloy/issues/1924))
- Migrate eip1898 tests ([#1922](https://github.com/alloy-rs/alloy/issues/1922))

## [0.9.2](https://github.com/alloy-rs/alloy/releases/tag/v0.9.2) - 2025-01-03

### Bug Fixes

- [eip7251] Update contract address and bytecode ([#1877](https://github.com/alloy-rs/alloy/issues/1877))
- Skip empty request objects ([#1873](https://github.com/alloy-rs/alloy/issues/1873))

### Features

- Sort and skip empty requests for hash ([#1878](https://github.com/alloy-rs/alloy/issues/1878))

### Miscellaneous Tasks

- Release 0.9.2

## [0.9.1](https://github.com/alloy-rs/alloy/releases/tag/v0.9.1) - 2024-12-30

### Miscellaneous Tasks

- Release 0.9.1
- Add history serve window ([#1865](https://github.com/alloy-rs/alloy/issues/1865))

## [0.9.0](https://github.com/alloy-rs/alloy/releases/tag/v0.9.0) - 2024-12-30

### Bug Fixes

- [alloy-eips] `SimpleCoder::decode_one()` should return `Ok(None)` ([#1818](https://github.com/alloy-rs/alloy/issues/1818))

### Features

- EIP-7840 ([#1828](https://github.com/alloy-rs/alloy/issues/1828))
- [pectra] Revert EIP-7742 ([#1807](https://github.com/alloy-rs/alloy/issues/1807))

### Other

- [Feature] update Display implementation on BlockNumberOrTag ([#1857](https://github.com/alloy-rs/alloy/issues/1857))
- [Bug] Request predeploy codes have diverged ([#1845](https://github.com/alloy-rs/alloy/issues/1845))
- Update contract bytecode & address ([#1838](https://github.com/alloy-rs/alloy/issues/1838))
- Update `CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS` ([#1836](https://github.com/alloy-rs/alloy/issues/1836))
- Update `WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS` ([#1834](https://github.com/alloy-rs/alloy/issues/1834))

## [0.8.3](https://github.com/alloy-rs/alloy/releases/tag/v0.8.3) - 2024-12-20

### Miscellaneous Tasks

- Release 0.8.3

## [0.8.2](https://github.com/alloy-rs/alloy/releases/tag/v0.8.2) - 2024-12-19

### Miscellaneous Tasks

- Release 0.8.2

## [0.8.1](https://github.com/alloy-rs/alloy/releases/tag/v0.8.1) - 2024-12-16

### Features

- [relay] ExecutionRequestsV4 with eip7685::Requests conversion ([#1787](https://github.com/alloy-rs/alloy/issues/1787))
- Add requests with capacity ([#1794](https://github.com/alloy-rs/alloy/issues/1794))

### Miscellaneous Tasks

- Release 0.8.1
- Port calc block gas limit ([#1798](https://github.com/alloy-rs/alloy/issues/1798))
- Add helper for loading custom trusted setup ([#1779](https://github.com/alloy-rs/alloy/issues/1779))

### Other

- Calc_blob_gasprice made const ([#1788](https://github.com/alloy-rs/alloy/issues/1788))

## [0.8.0](https://github.com/alloy-rs/alloy/releases/tag/v0.8.0) - 2024-12-10

### Features

- Add arbitrary for alloy types ([#1777](https://github.com/alloy-rs/alloy/issues/1777))
- EIP-7691 ([#1762](https://github.com/alloy-rs/alloy/issues/1762))

### Miscellaneous Tasks

- Release 0.8.0 ([#1778](https://github.com/alloy-rs/alloy/issues/1778))
- Derive Copy for BlockWithParent ([#1776](https://github.com/alloy-rs/alloy/issues/1776))
- Improve Display and Debug for BlockId ([#1765](https://github.com/alloy-rs/alloy/issues/1765))

## [0.7.3](https://github.com/alloy-rs/alloy/releases/tag/v0.7.3) - 2024-12-05

### Miscellaneous Tasks

- Release 0.7.3

## [Unreleased](https://github.com/alloy-rs/alloy/compare/v0.7.0...HEAD)

### Bug Fixes

- Adjust EIP-7742 to latest spec ([#1713](https://github.com/alloy-rs/alloy/issues/1713))
- Omit empty requests ([#1706](https://github.com/alloy-rs/alloy/issues/1706))
- Use B256::new instead of from ([#1701](https://github.com/alloy-rs/alloy/issues/1701))

### Dependencies

- [general] Bump MSRV to 1.81, use `core::error::Error` on `no-std` compatible crates ([#1552](https://github.com/alloy-rs/alloy/issues/1552))

### Documentation

- Update docs for eip7685 `Requests` ([#1714](https://github.com/alloy-rs/alloy/issues/1714))

### Features

- Impl `Encodable2718` for `ReceiptWithBloom` ([#1719](https://github.com/alloy-rs/alloy/issues/1719))
- EIP-7685 requests helpers ([#1699](https://github.com/alloy-rs/alloy/issues/1699))

### Miscellaneous Tasks

- Release 0.7.2 ([#1729](https://github.com/alloy-rs/alloy/issues/1729))

## [0.7.0](https://github.com/alloy-rs/alloy/releases/tag/v0.7.0) - 2024-11-28

### Bug Fixes

- EIP-7742 fixes ([#1697](https://github.com/alloy-rs/alloy/issues/1697))

### Features

- [eips] Make prague field an enum ([#1574](https://github.com/alloy-rs/alloy/issues/1574))
- EIP-7742 ([#1600](https://github.com/alloy-rs/alloy/issues/1600))

### Miscellaneous Tasks

- Release 0.7.0
- EIP-7685 changes ([#1599](https://github.com/alloy-rs/alloy/issues/1599))

### Other

- Add `BlockWithParent` ([#1650](https://github.com/alloy-rs/alloy/issues/1650))

## [0.6.4](https://github.com/alloy-rs/alloy/releases/tag/v0.6.4) - 2024-11-12

### Miscellaneous Tasks

- Release 0.6.4

## [0.6.3](https://github.com/alloy-rs/alloy/releases/tag/v0.6.3) - 2024-11-12

### Miscellaneous Tasks

- Release 0.6.3
- Release 0.6.2 ([#1632](https://github.com/alloy-rs/alloy/issues/1632))

## [0.6.1](https://github.com/alloy-rs/alloy/releases/tag/v0.6.1) - 2024-11-06

### Miscellaneous Tasks

- Release 0.6.1

## [0.6.0](https://github.com/alloy-rs/alloy/releases/tag/v0.6.0) - 2024-11-06

### Bug Fixes

- Add more rlp correctness checks ([#1595](https://github.com/alloy-rs/alloy/issues/1595))
- Make a sensible encoding api ([#1496](https://github.com/alloy-rs/alloy/issues/1496))

### Documentation

- Expand on what `Requests` contains ([#1564](https://github.com/alloy-rs/alloy/issues/1564))

### Features

- [eips] Indexed Blob Hash ([#1526](https://github.com/alloy-rs/alloy/issues/1526))

### Miscellaneous Tasks

- Release 0.6.0
- Make withdrawals pub ([#1623](https://github.com/alloy-rs/alloy/issues/1623))
- Fix some compile issues for no-std test ([#1606](https://github.com/alloy-rs/alloy/issues/1606))

### Other

- Add missing unit test for `MIN_PROTOCOL_BASE_FEE` ([#1558](https://github.com/alloy-rs/alloy/issues/1558))
- Rm `BEACON_CONSENSUS_REORG_UNWIND_DEPTH` ([#1556](https://github.com/alloy-rs/alloy/issues/1556))
- Add unit tests to secure all conversions and impl ([#1544](https://github.com/alloy-rs/alloy/issues/1544))

## [0.5.4](https://github.com/alloy-rs/alloy/releases/tag/v0.5.4) - 2024-10-23

### Bug Fixes

- Sidecar rlp decoding ([#1549](https://github.com/alloy-rs/alloy/issues/1549))

### Miscellaneous Tasks

- Release 0.5.4

### Other

- Add unit test for `amount_wei` `Withdrawal` ([#1551](https://github.com/alloy-rs/alloy/issues/1551))

## [0.5.3](https://github.com/alloy-rs/alloy/releases/tag/v0.5.3) - 2024-10-22

### Bug Fixes

- Correct implementations of Encodable and Decodable for sidecars ([#1528](https://github.com/alloy-rs/alloy/issues/1528))

### Miscellaneous Tasks

- Release 0.5.3

### Other

- Impl `From<RpcBlockHash>` for `BlockId` ([#1539](https://github.com/alloy-rs/alloy/issues/1539))
- Add unit tests and reduce paths ([#1531](https://github.com/alloy-rs/alloy/issues/1531))

## [0.5.2](https://github.com/alloy-rs/alloy/releases/tag/v0.5.2) - 2024-10-18

### Miscellaneous Tasks

- Release 0.5.2

## [0.5.1](https://github.com/alloy-rs/alloy/releases/tag/v0.5.1) - 2024-10-18

### Miscellaneous Tasks

- Release 0.5.1
- Add empty requests constant ([#1519](https://github.com/alloy-rs/alloy/issues/1519))
- Remove 7685 request variants ([#1515](https://github.com/alloy-rs/alloy/issues/1515))
- Remove redundant cfgs ([#1516](https://github.com/alloy-rs/alloy/issues/1516))

## [0.5.0](https://github.com/alloy-rs/alloy/releases/tag/v0.5.0) - 2024-10-18

### Bug Fixes

- [eips] Blob Sidecar Item Serde ([#1441](https://github.com/alloy-rs/alloy/issues/1441))

### Features

- [eip4895] Implement `Withdrawals` ([#1462](https://github.com/alloy-rs/alloy/issues/1462))
- Port generate_blob_sidecar ([#1511](https://github.com/alloy-rs/alloy/issues/1511))
- [eips] Arbitrary BaseFeeParams ([#1432](https://github.com/alloy-rs/alloy/issues/1432))
- `Encodable2718::network_len` ([#1431](https://github.com/alloy-rs/alloy/issues/1431))
- Add helper from impl ([#1407](https://github.com/alloy-rs/alloy/issues/1407))

### Miscellaneous Tasks

- Release 0.5.0
- Update pectra system contracts bytecodes & addresses ([#1512](https://github.com/alloy-rs/alloy/issues/1512))
- Refactor some match with same arms ([#1463](https://github.com/alloy-rs/alloy/issues/1463))
- Update eip-7251 bytecode and address ([#1380](https://github.com/alloy-rs/alloy/issues/1380))
- Remove redundant else ([#1468](https://github.com/alloy-rs/alloy/issues/1468))
- Some small improvements ([#1461](https://github.com/alloy-rs/alloy/issues/1461))

### Other

- Update fn encoded_2718 ([#1475](https://github.com/alloy-rs/alloy/issues/1475))
- Add unit tests for `ConsolidationRequest` ([#1497](https://github.com/alloy-rs/alloy/issues/1497))
- Add unit tests for `WithdrawalRequest` ([#1472](https://github.com/alloy-rs/alloy/issues/1472))
- Add more unit tests ([#1464](https://github.com/alloy-rs/alloy/issues/1464))
- Revert test: update test cases with addresses ([#1358](https://github.com/alloy-rs/alloy/issues/1358)) ([#1444](https://github.com/alloy-rs/alloy/issues/1444))

## [0.4.2](https://github.com/alloy-rs/alloy/releases/tag/v0.4.2) - 2024-10-01

### Miscellaneous Tasks

- Release 0.4.2

## [0.4.1](https://github.com/alloy-rs/alloy/releases/tag/v0.4.1) - 2024-10-01

### Bug Fixes

- Safe match for next base fee ([#1399](https://github.com/alloy-rs/alloy/issues/1399))

### Features

- [consensus] Bincode compatibility for EIP-7702 ([#1404](https://github.com/alloy-rs/alloy/issues/1404))

### Miscellaneous Tasks

- Release 0.4.1

## [0.4.0](https://github.com/alloy-rs/alloy/releases/tag/v0.4.0) - 2024-09-30

### Bug Fixes

- Support u64 hex from str for BlockId ([#1396](https://github.com/alloy-rs/alloy/issues/1396))
- Advance buffer during 2718 decoding ([#1367](https://github.com/alloy-rs/alloy/issues/1367))
- `Error::source` for `Eip2718Error` ([#1361](https://github.com/alloy-rs/alloy/issues/1361))

### Features

- Impl From<Eip2718Error> for alloy_rlp::Error ([#1359](https://github.com/alloy-rs/alloy/issues/1359))
- Blob Tx Sidecar Iterator ([#1334](https://github.com/alloy-rs/alloy/issues/1334))

### Miscellaneous Tasks

- Release 0.4.0
- Use std::error

### Other

- Make `Header` blob fees u64 ([#1377](https://github.com/alloy-rs/alloy/issues/1377))
- Make `Header` gas limit u64 ([#1333](https://github.com/alloy-rs/alloy/issues/1333))

### Testing

- Update test cases with addresses ([#1358](https://github.com/alloy-rs/alloy/issues/1358))

## [0.3.6](https://github.com/alloy-rs/alloy/releases/tag/v0.3.6) - 2024-09-18

### Features

- [rpc-types-beacon] `SignedBidSubmissionV4` ([#1303](https://github.com/alloy-rs/alloy/issues/1303))
- Add blob and proof v1 ([#1300](https://github.com/alloy-rs/alloy/issues/1300))

### Miscellaneous Tasks

- Release 0.3.6

## [0.3.5](https://github.com/alloy-rs/alloy/releases/tag/v0.3.5) - 2024-09-13

### Miscellaneous Tasks

- Release 0.3.5

## [0.3.4](https://github.com/alloy-rs/alloy/releases/tag/v0.3.4) - 2024-09-13

### Features

- Add serde for NumHash ([#1277](https://github.com/alloy-rs/alloy/issues/1277))

### Miscellaneous Tasks

- Release 0.3.4
- Swap `BlockHashOrNumber` alias and struct name ([#1270](https://github.com/alloy-rs/alloy/issues/1270))

## [0.3.3](https://github.com/alloy-rs/alloy/releases/tag/v0.3.3) - 2024-09-10

### Miscellaneous Tasks

- Release 0.3.3
- Swap BlockNumHash alias and struct name ([#1265](https://github.com/alloy-rs/alloy/issues/1265))

## [0.3.2](https://github.com/alloy-rs/alloy/releases/tag/v0.3.2) - 2024-09-09

### Miscellaneous Tasks

- Release 0.3.2
- Add aliases for Num Hash ([#1253](https://github.com/alloy-rs/alloy/issues/1253))
- [eip1898] Display `RpcBlockHash` ([#1242](https://github.com/alloy-rs/alloy/issues/1242))
- Optional derive more ([#1239](https://github.com/alloy-rs/alloy/issues/1239))

## [0.3.1](https://github.com/alloy-rs/alloy/releases/tag/v0.3.1) - 2024-09-02

### Bug Fixes

- [eips] No-std compat ([#1222](https://github.com/alloy-rs/alloy/issues/1222))

### Miscellaneous Tasks

- Release 0.3.1

## [0.3.0](https://github.com/alloy-rs/alloy/releases/tag/v0.3.0) - 2024-08-28

### Bug Fixes

- [doc] Correct order of fields ([#1139](https://github.com/alloy-rs/alloy/issues/1139))
- Correctly trim eip7251 bytecode ([#1105](https://github.com/alloy-rs/alloy/issues/1105))
- [eips] Make SignedAuthorizationList arbitrary less fallible ([#1084](https://github.com/alloy-rs/alloy/issues/1084))
- Require storageKeys value broken bincode serialization from [#955](https://github.com/alloy-rs/alloy/issues/955) ([#1058](https://github.com/alloy-rs/alloy/issues/1058))
- Cargo fmt ([#1044](https://github.com/alloy-rs/alloy/issues/1044))
- [eip7702] Add correct rlp decode/encode ([#1034](https://github.com/alloy-rs/alloy/issues/1034))

### Dependencies

- Rm 2930 and 7702 - use alloy-rs/eips ([#1181](https://github.com/alloy-rs/alloy/issues/1181))
- Bump core and rm ssz feat ([#1167](https://github.com/alloy-rs/alloy/issues/1167))
- [deps] Bump some deps ([#1141](https://github.com/alloy-rs/alloy/issues/1141))

### Features

- [eip] Make 7702 auth recovery fallible ([#1082](https://github.com/alloy-rs/alloy/issues/1082))
- Add authorization list to rpc transaction and tx receipt types ([#1051](https://github.com/alloy-rs/alloy/issues/1051))
- Generate valid signed auth signatures ([#1041](https://github.com/alloy-rs/alloy/issues/1041))
- Add arbitrary to auth ([#1036](https://github.com/alloy-rs/alloy/issues/1036))
- Add hash for 7702 ([#1037](https://github.com/alloy-rs/alloy/issues/1037))

### Miscellaneous Tasks

- Release 0.3.0
- Clippy für docs ([#1194](https://github.com/alloy-rs/alloy/issues/1194))
- [eip7702] Devnet3 changes ([#1056](https://github.com/alloy-rs/alloy/issues/1056))
- Release 0.2.1
- Release 0.2.0
- Make auth mandatory in recovered auth ([#1047](https://github.com/alloy-rs/alloy/issues/1047))

### Other

- Add conversion from BlockHashOrNumber to BlockId ([#1127](https://github.com/alloy-rs/alloy/issues/1127))
- Add `AccessListResult` type (EIP-2930) ([#1110](https://github.com/alloy-rs/alloy/issues/1110))

### Styling

- Remove proptest in all crates and Arbitrary derives ([#966](https://github.com/alloy-rs/alloy/issues/966))

## [0.1.4](https://github.com/alloy-rs/alloy/releases/tag/v0.1.4) - 2024-07-08

### Features

- Add consolidation requests to v4 payload ([#1013](https://github.com/alloy-rs/alloy/issues/1013))
- [eip1559] Support Optimism Canyon hardfork ([#1010](https://github.com/alloy-rs/alloy/issues/1010))
- Impl `From<RpcBlockHash>` for `BlockHashOrNumber` ([#980](https://github.com/alloy-rs/alloy/issues/980))

### Miscellaneous Tasks

- Release 0.1.4
- Add helper functions for destructuring auth types ([#1022](https://github.com/alloy-rs/alloy/issues/1022))
- Clean up 7702 encoding ([#1000](https://github.com/alloy-rs/alloy/issues/1000))

### Testing

- Add missing unit test for op `calc_next_block_base_fee` ([#1008](https://github.com/alloy-rs/alloy/issues/1008))

## [0.1.3](https://github.com/alloy-rs/alloy/releases/tag/v0.1.3) - 2024-06-25

### Bug Fixes

- Deserialization of null storage keys in AccessListItem ([#955](https://github.com/alloy-rs/alloy/issues/955))

### Dependencies

- [eips] Make `alloy-serde` optional under `serde` ([#948](https://github.com/alloy-rs/alloy/issues/948))

### Features

- Add eip-7702 helpers ([#950](https://github.com/alloy-rs/alloy/issues/950))
- Add eip-7251 system contract address/code ([#956](https://github.com/alloy-rs/alloy/issues/956))

### Miscellaneous Tasks

- Release 0.1.3
- [eips] Add serde to Authorization types ([#964](https://github.com/alloy-rs/alloy/issues/964))
- [eips] Make `sha2` optional, add `kzg-sidecar` feature ([#949](https://github.com/alloy-rs/alloy/issues/949))

## [0.1.2](https://github.com/alloy-rs/alloy/releases/tag/v0.1.2) - 2024-06-19

### Documentation

- Update alloy-eips supported eip list ([#942](https://github.com/alloy-rs/alloy/issues/942))
- Touch up docs, TODOs ([#918](https://github.com/alloy-rs/alloy/issues/918))
- Add per-crate changelogs ([#914](https://github.com/alloy-rs/alloy/issues/914))

### Features

- Add eip-7251 consolidation request ([#919](https://github.com/alloy-rs/alloy/issues/919))
- Add `BlockId::as_u64` ([#916](https://github.com/alloy-rs/alloy/issues/916))

### Miscellaneous Tasks

- Release 0.1.2
- Update eip-2935 bytecode and address ([#934](https://github.com/alloy-rs/alloy/issues/934))
- Update changelogs for v0.1.1 ([#922](https://github.com/alloy-rs/alloy/issues/922))
- Add docs.rs metadata to all manifests ([#917](https://github.com/alloy-rs/alloy/issues/917))

## [0.1.1](https://github.com/alloy-rs/alloy/releases/tag/v0.1.1) - 2024-06-17

### Bug Fixes

- Non_exhaustive for 2718 error ([#837](https://github.com/alloy-rs/alloy/issues/837))
- Add proptest derives back ([#797](https://github.com/alloy-rs/alloy/issues/797))
- Serde rename camelcase ([#748](https://github.com/alloy-rs/alloy/issues/748))
- Correct exitV1 type ([#567](https://github.com/alloy-rs/alloy/issues/567))
- Infinite loop while decoding a list of transactions ([#432](https://github.com/alloy-rs/alloy/issues/432))
- Use enveloped encoding for typed transactions ([#239](https://github.com/alloy-rs/alloy/issues/239))
- [`eips`/`consensus`] Correctly decode txs on `TxEnvelope` ([#148](https://github.com/alloy-rs/alloy/issues/148))

### Dependencies

- Deduplicate AccessList and Withdrawals types ([#324](https://github.com/alloy-rs/alloy/issues/324))
- Alloy-consensus crate ([#83](https://github.com/alloy-rs/alloy/issues/83))

### Documentation

- Update descriptions and top level summary ([#128](https://github.com/alloy-rs/alloy/issues/128))

### Features

- Move `{,With}OtherFields` to serde crate ([#892](https://github.com/alloy-rs/alloy/issues/892))
- Derive `Default` for `WithdrawalRequest` and `DepositRequest` ([#867](https://github.com/alloy-rs/alloy/issues/867))
- [serde] Deprecate individual num::* for a generic `quantity` module ([#855](https://github.com/alloy-rs/alloy/issues/855))
- [eips] EIP-2935 history storage contract ([#747](https://github.com/alloy-rs/alloy/issues/747))
- Rlp enc/dec for requests ([#728](https://github.com/alloy-rs/alloy/issues/728))
- [consensus, eips] EIP-7002 system contract ([#727](https://github.com/alloy-rs/alloy/issues/727))
- Add eth mainnet EL requests envelope ([#707](https://github.com/alloy-rs/alloy/issues/707))
- Add eip-7685 enc/decode traits ([#704](https://github.com/alloy-rs/alloy/issues/704))
- Rlp for eip-7002 requests ([#705](https://github.com/alloy-rs/alloy/issues/705))
- Manual blob deserialize ([#696](https://github.com/alloy-rs/alloy/issues/696))
- Derive arbitrary for BlobTransactionSidecar ([#679](https://github.com/alloy-rs/alloy/issues/679))
- Use alloy types for BlobTransactionSidecar ([#673](https://github.com/alloy-rs/alloy/issues/673))
- Add prague engine types ([#557](https://github.com/alloy-rs/alloy/issues/557))
- Add BaseFeeParams::new ([#525](https://github.com/alloy-rs/alloy/issues/525))
- Port helpers for accesslist ([#508](https://github.com/alloy-rs/alloy/issues/508))
- Joinable transaction fillers ([#426](https://github.com/alloy-rs/alloy/issues/426))
- Serde for consensus tx types ([#361](https://github.com/alloy-rs/alloy/issues/361))
- 4844 SidecarBuilder ([#250](https://github.com/alloy-rs/alloy/issues/250))
- Support no_std for `alloy-eips` ([#181](https://github.com/alloy-rs/alloy/issues/181))
- [providers] Event, polling and streaming methods ([#274](https://github.com/alloy-rs/alloy/issues/274))
- Network abstraction and transaction builder ([#190](https://github.com/alloy-rs/alloy/issues/190))
- [`consensus`] Add extra EIP-4844 types needed ([#229](https://github.com/alloy-rs/alloy/issues/229))

### Miscellaneous Tasks

- Update EIP7002 withdrawal requests based on spec ([#885](https://github.com/alloy-rs/alloy/issues/885))
- [other] Use type aliases where possible to improve clarity  ([#859](https://github.com/alloy-rs/alloy/issues/859))
- [eips] Compile tests with default features ([#860](https://github.com/alloy-rs/alloy/issues/860))
- [docs] Crate completeness and fix typos ([#861](https://github.com/alloy-rs/alloy/issues/861))
- [docs] Add doc aliases ([#843](https://github.com/alloy-rs/alloy/issues/843))
- Add Into for WithOtherFields in rpc types ([#813](https://github.com/alloy-rs/alloy/issues/813))
- Fix remaining warnings, add TODO for proptest-derive ([#819](https://github.com/alloy-rs/alloy/issues/819))
- Fix warnings, check-cfg ([#776](https://github.com/alloy-rs/alloy/issues/776))
- Rename deposit receipt to deposit request ([#693](https://github.com/alloy-rs/alloy/issues/693))
- Move blob validation to sidecar ([#677](https://github.com/alloy-rs/alloy/issues/677))
- Replace `ExitV1` with `WithdrawalRequest` ([#672](https://github.com/alloy-rs/alloy/issues/672))
- Move BlockId type to alloy-eip ([#565](https://github.com/alloy-rs/alloy/issues/565))
- Clippy, warnings ([#504](https://github.com/alloy-rs/alloy/issues/504))
- Add helper for next block base fee ([#494](https://github.com/alloy-rs/alloy/issues/494))
- Clean up kzg and features ([#386](https://github.com/alloy-rs/alloy/issues/386))
- Error when missing to field in transaction conversion ([#365](https://github.com/alloy-rs/alloy/issues/365))
- Clippy ([#251](https://github.com/alloy-rs/alloy/issues/251))

### Other

- [Fix] use Eip2718Error, add docs on different encodings ([#869](https://github.com/alloy-rs/alloy/issues/869))
- Add clippy at workspace level ([#766](https://github.com/alloy-rs/alloy/issues/766))
- Arbitrary Sidecar implementation + build. Closes [#680](https://github.com/alloy-rs/alloy/issues/680). ([#708](https://github.com/alloy-rs/alloy/issues/708))
- Use Self instead of BlockNumberOrTag ([#754](https://github.com/alloy-rs/alloy/issues/754))
- Use Self when possible ([#711](https://github.com/alloy-rs/alloy/issues/711))
- Small refactor ([#652](https://github.com/alloy-rs/alloy/issues/652))
- Move block hash types to alloy-eips ([#639](https://github.com/alloy-rs/alloy/issues/639))
- Add arbitrary derive for Withdrawal ([#501](https://github.com/alloy-rs/alloy/issues/501))
- Extension ([#474](https://github.com/alloy-rs/alloy/issues/474))
- Derive arbitrary for rpc `Header` and `Transaction` ([#458](https://github.com/alloy-rs/alloy/issues/458))
- Added MAINNET_KZG_TRUSTED_SETUP ([#385](https://github.com/alloy-rs/alloy/issues/385))
- Check no_std in CI ([#367](https://github.com/alloy-rs/alloy/issues/367))

### Refactor

- Clean up legacy serde helpers ([#624](https://github.com/alloy-rs/alloy/issues/624))

### Styling

- [Blocked] Update TransactionRequest's `to` field to TxKind ([#553](https://github.com/alloy-rs/alloy/issues/553))
- Sort derives ([#499](https://github.com/alloy-rs/alloy/issues/499))
- [Feature] Move Mainnet KZG group and Lazy<KzgSettings> ([#368](https://github.com/alloy-rs/alloy/issues/368))

[`alloy`]: https://crates.io/crates/alloy
[alloy]: https://crates.io/crates/alloy
[`alloy-core`]: https://crates.io/crates/alloy-core
[alloy-core]: https://crates.io/crates/alloy-core
[`alloy-consensus`]: https://crates.io/crates/alloy-consensus
[alloy-consensus]: https://crates.io/crates/alloy-consensus
[`alloy-contract`]: https://crates.io/crates/alloy-contract
[alloy-contract]: https://crates.io/crates/alloy-contract
[`alloy-eips`]: https://crates.io/crates/alloy-eips
[alloy-eips]: https://crates.io/crates/alloy-eips
[`alloy-genesis`]: https://crates.io/crates/alloy-genesis
[alloy-genesis]: https://crates.io/crates/alloy-genesis
[`alloy-json-rpc`]: https://crates.io/crates/alloy-json-rpc
[alloy-json-rpc]: https://crates.io/crates/alloy-json-rpc
[`alloy-network`]: https://crates.io/crates/alloy-network
[alloy-network]: https://crates.io/crates/alloy-network
[`alloy-node-bindings`]: https://crates.io/crates/alloy-node-bindings
[alloy-node-bindings]: https://crates.io/crates/alloy-node-bindings
[`alloy-provider`]: https://crates.io/crates/alloy-provider
[alloy-provider]: https://crates.io/crates/alloy-provider
[`alloy-pubsub`]: https://crates.io/crates/alloy-pubsub
[alloy-pubsub]: https://crates.io/crates/alloy-pubsub
[`alloy-rpc-client`]: https://crates.io/crates/alloy-rpc-client
[alloy-rpc-client]: https://crates.io/crates/alloy-rpc-client
[`alloy-rpc-types`]: https://crates.io/crates/alloy-rpc-types
[alloy-rpc-types]: https://crates.io/crates/alloy-rpc-types
[`alloy-rpc-types-anvil`]: https://crates.io/crates/alloy-rpc-types-anvil
[alloy-rpc-types-anvil]: https://crates.io/crates/alloy-rpc-types-anvil
[`alloy-rpc-types-beacon`]: https://crates.io/crates/alloy-rpc-types-beacon
[alloy-rpc-types-beacon]: https://crates.io/crates/alloy-rpc-types-beacon
[`alloy-rpc-types-engine`]: https://crates.io/crates/alloy-rpc-types-engine
[alloy-rpc-types-engine]: https://crates.io/crates/alloy-rpc-types-engine
[`alloy-rpc-types-eth`]: https://crates.io/crates/alloy-rpc-types-eth
[alloy-rpc-types-eth]: https://crates.io/crates/alloy-rpc-types-eth
[`alloy-rpc-types-trace`]: https://crates.io/crates/alloy-rpc-types-trace
[alloy-rpc-types-trace]: https://crates.io/crates/alloy-rpc-types-trace
[`alloy-serde`]: https://crates.io/crates/alloy-serde
[alloy-serde]: https://crates.io/crates/alloy-serde
[`alloy-signer`]: https://crates.io/crates/alloy-signer
[alloy-signer]: https://crates.io/crates/alloy-signer
[`alloy-signer-aws`]: https://crates.io/crates/alloy-signer-aws
[alloy-signer-aws]: https://crates.io/crates/alloy-signer-aws
[`alloy-signer-gcp`]: https://crates.io/crates/alloy-signer-gcp
[alloy-signer-gcp]: https://crates.io/crates/alloy-signer-gcp
[`alloy-signer-ledger`]: https://crates.io/crates/alloy-signer-ledger
[alloy-signer-ledger]: https://crates.io/crates/alloy-signer-ledger
[`alloy-signer-local`]: https://crates.io/crates/alloy-signer-local
[alloy-signer-local]: https://crates.io/crates/alloy-signer-local
[`alloy-signer-trezor`]: https://crates.io/crates/alloy-signer-trezor
[alloy-signer-trezor]: https://crates.io/crates/alloy-signer-trezor
[`alloy-signer-wallet`]: https://crates.io/crates/alloy-signer-wallet
[alloy-signer-wallet]: https://crates.io/crates/alloy-signer-wallet
[`alloy-transport`]: https://crates.io/crates/alloy-transport
[alloy-transport]: https://crates.io/crates/alloy-transport
[`alloy-transport-http`]: https://crates.io/crates/alloy-transport-http
[alloy-transport-http]: https://crates.io/crates/alloy-transport-http
[`alloy-transport-ipc`]: https://crates.io/crates/alloy-transport-ipc
[alloy-transport-ipc]: https://crates.io/crates/alloy-transport-ipc
[`alloy-transport-ws`]: https://crates.io/crates/alloy-transport-ws
[alloy-transport-ws]: https://crates.io/crates/alloy-transport-ws

<!-- generated by git-cliff -->
