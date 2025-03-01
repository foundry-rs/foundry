# Changelog [![crates.io][crate-badge]][crate]

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog] and this project adheres to
[Semantic Versioning].

## [Unreleased]

## [1.3.0] - 2023-08-20

### Added

- `num` module containing conditional aliases to numeric types.

## [1.2.0] - 2023-05-09

### Added

- `if let` pattern matching in [`condval!`].

## [1.1.0] - 2023-04-25

### Added

- [`condval!`] macro to construct [conditionally-typed][CondType] values.

## 1.0.0 - 2023-04-18

### Added

- [`CondType`][CondType] type alias that is determined by a boolean condition,
  just like [`std::conditional_t` in C++](https://en.cppreference.com/w/cpp/types/conditional).

[crate]:       https://crates.io/crates/condtype
[crate-badge]: https://img.shields.io/crates/v/condtype.svg

[Keep a Changelog]:    http://keepachangelog.com/en/1.0.0/
[Semantic Versioning]: http://semver.org/spec/v2.0.0.html

[Unreleased]: https://github.com/nvzqz/condtype/compare/v1.3.0...HEAD
[1.3.0]: https://github.com/nvzqz/condtype/compare/v1.2.0...v1.3.0
[1.2.0]: https://github.com/nvzqz/condtype/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/nvzqz/condtype/compare/v1.0.0...v1.1.0

[CondType]:   https://docs.rs/condtype/latest/condtype/type.CondType.html
[`condval!`]: https://docs.rs/condtype/latest/condtype/macro.condval.html
