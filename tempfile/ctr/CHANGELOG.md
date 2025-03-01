# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## 0.9.2 (2022-09-30)
### Changed
- Implement `Clone` directly for `CtrCore`, so it would work with non-`Clone` flavors ([#24])

[#24]: https://github.com/RustCrypto/block-modes/pull/24

## 0.9.1 (2022-02-17)
### Fixed
- Minimal versions build ([#9])

[#9]: https://github.com/RustCrypto/block-modes/pull/9

## 0.9.0 (2022-02-10)
### Changed
- Update `cipher` dependency to v0.4 and move crate
to the [RustCrypto/block-modes] repository ([#2])

[#2]: https://github.com/RustCrypto/block-modes/pull/2
[RustCrypto/block-modes]: https://github.com/RustCrypto/block-modes

## 0.8.0 (2021-07-08)
### Changed
- Make implementation generic over block size (previously it
was generic only over 128-bit block ciphers). Breaking changes
in the `CtrFlavor` API. ([#252]).

[#252]: https://github.com/RustCrypto/stream-ciphers/pull/252

## 0.7.0 (2020-04-29)
### Changed
- Generic implementation of CTR ([#195])
- Removed `Ctr32LE` mask bit ([#197])
- Bump `cipher` dependency to v0.3 ([#226])

[#195]: https://github.com/RustCrypto/stream-ciphers/pull/195
[#197]: https://github.com/RustCrypto/stream-ciphers/pull/197
[#226]: https://github.com/RustCrypto/stream-ciphers/pull/226

## 0.6.0 (2020-10-16)
### Added
- `Ctr32BE` and `Ctr32LE` ([#170])

### Changed
- Replace `block-cipher`/`stream-cipher` with `cipher` crate ([#177])

[#177]: https://github.com/RustCrypto/stream-ciphers/pull/177
[#170]: https://github.com/RustCrypto/stream-ciphers/pull/170

## 0.5.0 (2020-08-26)
### Changed
- Bump `stream-cipher` dependency to v0.7, implement the `FromBlockCipher` trait ([#161], [#164])

[#161]: https://github.com/RustCrypto/stream-ciphers/pull/161
[#164]: https://github.com/RustCrypto/stream-ciphers/pull/164

## 0.4.0 (2020-06-06)
### Changed
- Upgrade to the `stream-cipher` v0.4 crate ([#116], [#138])
- Upgrade to Rust 2018 edition ([#116])

[#138]: https://github.com/RustCrypto/stream-ciphers/pull/138
[#116]: https://github.com/RustCrypto/stream-ciphers/pull/121

## 0.3.2 (2019-03-11)

## 0.3.0 (2018-11-01)

## 0.2.0 (2018-10-13)

## 0.1.1 (2018-10-13)

## 0.1.0 (2018-07-30)
