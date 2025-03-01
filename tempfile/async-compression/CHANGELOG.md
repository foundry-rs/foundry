# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

## [0.4.18](https://github.com/Nullus157/async-compression/compare/v0.4.17...v0.4.18) - 2024-11-23

### Fixed

- Adjust `Level::Precise` clamp range for flate2.

## [0.4.17](https://github.com/Nullus157/async-compression/compare/v0.4.16...v0.4.17) - 2024-10-20

### Fixed

- Fix occasional panics when consuming from pending buffers.

## [0.4.16](https://github.com/Nullus157/async-compression/compare/v0.4.15...v0.4.16) - 2024-10-16

### Other

- Implement pass-through `AsyncBufRead` on write-based encoders & decoders.

## [0.4.15](https://github.com/Nullus157/async-compression/compare/v0.4.14...v0.4.15) - 2024-10-13

### Feature
- Implement pass-through `AsyncRead` or `AsyncWrite` where appropriate.
- Relax `AsyncRead`/`AsyncWrite` bounds on `*::{get_ref, get_mut, get_pin_mut, into_inner}()` methods.

## [0.4.14](https://github.com/Nullus157/async-compression/compare/v0.4.13...v0.4.14) - 2024-10-10

### Fixed
- In Tokio-based decoders, attempt to decode from internal state even if nothing was read.

## [0.4.13](https://github.com/Nullus157/async-compression/compare/v0.4.12...v0.4.13) - 2024-10-02

### Feature
- Update `brotli` dependency to to `7`.

## [0.4.12](https://github.com/Nullus157/async-compression/compare/v0.4.11...v0.4.12) - 2024-07-21

### Feature
- Enable customizing Zstd decoding parameters.

## [0.4.11](https://github.com/Nullus157/async-compression/compare/v0.4.10...v0.4.11) - 2024-05-30

### Other
- Expose total_in/total_out from underlying flate2 encoder types.

## [0.4.10](https://github.com/Nullus157/async-compression/compare/v0.4.9...v0.4.10) - 2024-05-09

### Other
- *(deps)* update brotli requirement from 5.0 to 6.0 ([#274](https://github.com/Nullus157/async-compression/pull/274))
- Fix pipeline doc: Warn on unexpected cfgs instead of error ([#276](https://github.com/Nullus157/async-compression/pull/276))
- Update name of release-pr.yml
- Create release.yml
- Create release-pr.yml

## 0.4.9

 - bump dep brotli from 4.0 to 5.0

## 0.4.8

 - bump dep brotli from 3.3 to 4.0

## 0.4.7

- Flush available data in decoder even when there's no incoming input.

## 0.4.6

- Return errors instead of panicking in all encode and decode operations.

## 0.4.5

- Add `{Lzma, Xz}Decoder::with_mem_limit()` methods.

## 0.4.4

- Update `zstd` dependency to `0.13`.

## 0.4.3

- Implement `Default` for `brotli::EncoderParams`.

## 0.4.2

- Add top-level `brotli` module containing stable `brotli` crate wrapper types.
- Add `BrotliEncoder::with_quality_and_params()` constructors.
- Add `Deflate64Decoder` behind new crate feature `deflate64`.

## 0.4.1 - 2023-07-10

- Add `Zstd{Encoder,Decoder}::with_dict()` constructors.
- Add `zstdmt` crate feature that enables `zstd-safe/zstdmt`, allowing multi-threaded functionality to work as expected.

## 0.4.0 - 2023-05-10

- `Level::Precise` variant now takes a `i32` instead of `u32`.
- Add top-level `zstd` module containing stable `zstd` crate wrapper types.
- Add `ZstdEncoder::with_quality_and_params()` constructors.
- Update `zstd` dependency to `0.12`.
- Remove deprecated `stream`, `futures-bufread` and `futures-write` crate features.
- Remove Tokio 0.2.x and 0.3.x support (`tokio-02` and `tokio-03` crate features).

## 0.3.15 - 2022-10-08

- `Level::Default::into_zstd()` now returns zstd's default value `3`.
- Fix endianness when reading the `extra` field of a gzip header.
