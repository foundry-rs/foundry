# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.8.21](https://github.com/alloy-rs/core/releases/tag/v0.8.21) - 2025-02-10

### Bug Fixes

- [sol-macro] Call proc_macro_error handler manually ([#866](https://github.com/alloy-rs/core/issues/866))

### Features

- [`sol-macro-expander`] Increase resolve limit to 128 ([#864](https://github.com/alloy-rs/core/issues/864))

## [0.8.20](https://github.com/alloy-rs/core/releases/tag/v0.8.20) - 2025-02-02

### Miscellaneous Tasks

- Release 0.8.20

## [0.8.19](https://github.com/alloy-rs/core/releases/tag/v0.8.19) - 2025-01-15

### Documentation

- Enable some useful rustdoc features on docs.rs ([#850](https://github.com/alloy-rs/core/issues/850))

### Miscellaneous Tasks

- Release 0.8.19

## [0.8.18](https://github.com/alloy-rs/core/releases/tag/v0.8.18) - 2025-01-04

### Miscellaneous Tasks

- Release 0.8.18

## [0.8.17](https://github.com/alloy-rs/core/releases/tag/v0.8.17) - 2025-01-04

### Features

- [sol-macro] Translate contract types to address ([#842](https://github.com/alloy-rs/core/issues/842))
- [sol-macro] Evaluate array sizes ([#840](https://github.com/alloy-rs/core/issues/840))

### Miscellaneous Tasks

- Release 0.8.17

### Testing

- [sol-macro] Add a test for missing_docs ([#845](https://github.com/alloy-rs/core/issues/845))

## [0.8.16](https://github.com/alloy-rs/core/releases/tag/v0.8.16) - 2025-01-01

### Miscellaneous Tasks

- Release 0.8.16

## [0.8.15](https://github.com/alloy-rs/core/releases/tag/v0.8.15) - 2024-12-09

### Miscellaneous Tasks

- Release 0.8.15

### Other

- Remove unsafe code from macro expansions ([#818](https://github.com/alloy-rs/core/issues/818))

## [0.8.14](https://github.com/alloy-rs/core/releases/tag/v0.8.14) - 2024-11-28

### Miscellaneous Tasks

- Release 0.8.14

## [0.8.13](https://github.com/alloy-rs/core/releases/tag/v0.8.13) - 2024-11-26

### Bug Fixes

- [sol-macro] Expand all getter return types ([#812](https://github.com/alloy-rs/core/issues/812))

### Dependencies

- Remove cron schedule for deps.yml ([#808](https://github.com/alloy-rs/core/issues/808))

### Miscellaneous Tasks

- Release 0.8.13 ([#813](https://github.com/alloy-rs/core/issues/813))

## [0.8.12](https://github.com/alloy-rs/core/releases/tag/v0.8.12) - 2024-11-12

### Miscellaneous Tasks

- Release 0.8.12 ([#806](https://github.com/alloy-rs/core/issues/806))

## [0.8.11](https://github.com/alloy-rs/core/releases/tag/v0.8.11) - 2024-11-05

### Miscellaneous Tasks

- Release 0.8.11 ([#803](https://github.com/alloy-rs/core/issues/803))

## [0.8.10](https://github.com/alloy-rs/core/releases/tag/v0.8.10) - 2024-10-28

### Miscellaneous Tasks

- Release 0.8.10

## [0.8.9](https://github.com/alloy-rs/core/releases/tag/v0.8.9) - 2024-10-21

### Miscellaneous Tasks

- Release 0.8.9

## [0.8.8](https://github.com/alloy-rs/core/releases/tag/v0.8.8) - 2024-10-14

### Bug Fixes

- [alloy-sol-macro] Allow clippy::pub_underscore_fields on `sol!` output ([#770](https://github.com/alloy-rs/core/issues/770))

### Miscellaneous Tasks

- Release 0.8.8

## [0.8.7](https://github.com/alloy-rs/core/releases/tag/v0.8.7) - 2024-10-08

### Miscellaneous Tasks

- Release 0.8.7

## [0.8.6](https://github.com/alloy-rs/core/releases/tag/v0.8.6) - 2024-10-06

### Bug Fixes

- Fix lint `alloy-sol-macro-expander` ([#760](https://github.com/alloy-rs/core/issues/760))

### Miscellaneous Tasks

- Release 0.8.6

## [0.8.5](https://github.com/alloy-rs/core/releases/tag/v0.8.5) - 2024-09-25

### Miscellaneous Tasks

- Release 0.8.5

## [0.8.4](https://github.com/alloy-rs/core/releases/tag/v0.8.4) - 2024-09-25

### Bug Fixes

- [sol-types] Check signature in SolEvent if non-anonymous ([#741](https://github.com/alloy-rs/core/issues/741))

### Miscellaneous Tasks

- Release 0.8.4

## [0.8.3](https://github.com/alloy-rs/core/releases/tag/v0.8.3) - 2024-09-10

### Bug Fixes

- [sol-macro] Correctly determine whether event parameters are hashes ([#735](https://github.com/alloy-rs/core/issues/735))
- [sol-macro] Namespaced custom type resolution ([#731](https://github.com/alloy-rs/core/issues/731))
- Parse selector hashes in `sol` macro ([#730](https://github.com/alloy-rs/core/issues/730))

### Miscellaneous Tasks

- Release 0.8.3

## [0.8.2](https://github.com/alloy-rs/core/releases/tag/v0.8.2) - 2024-09-06

### Miscellaneous Tasks

- Release 0.8.2

## [0.8.1](https://github.com/alloy-rs/core/releases/tag/v0.8.1) - 2024-09-06

### Dependencies

- Bump MSRV to 1.79 ([#712](https://github.com/alloy-rs/core/issues/712))

### Miscellaneous Tasks

- Release 0.8.1
- Use proc-macro-error2 ([#723](https://github.com/alloy-rs/core/issues/723))

## [0.8.0](https://github.com/alloy-rs/core/releases/tag/v0.8.0) - 2024-08-21

### Features

- [sol-macro] Support namespaces ([#694](https://github.com/alloy-rs/core/issues/694))

### Miscellaneous Tasks

- Release 0.8.0

### Other

- Implement specific bit types for integers ([#677](https://github.com/alloy-rs/core/issues/677))

## [0.7.7](https://github.com/alloy-rs/core/releases/tag/v0.7.7) - 2024-07-08

### Documentation

- Add per-crate changelogs ([#669](https://github.com/alloy-rs/core/issues/669))

### Features

- IntoLogData ([#666](https://github.com/alloy-rs/core/issues/666))
- Add `abi_packed_encoded_size` ([#672](https://github.com/alloy-rs/core/issues/672))

### Miscellaneous Tasks

- Release 0.7.7
- Use workspace.lints ([#676](https://github.com/alloy-rs/core/issues/676))
- [sol-macro] Allow clippy all when emitting contract bytecode ([#674](https://github.com/alloy-rs/core/issues/674))

### Styling

- Sort derives ([#662](https://github.com/alloy-rs/core/issues/662))

## [0.7.5](https://github.com/alloy-rs/core/releases/tag/v0.7.5) - 2024-06-04

### Bug Fixes

- [sol-macro] Allow deriving `Default` on contracts ([#645](https://github.com/alloy-rs/core/issues/645))
- [sol-macro] Overridden event signatures ([#642](https://github.com/alloy-rs/core/issues/642))

### Documentation

- Update some READMEs ([#641](https://github.com/alloy-rs/core/issues/641))

### Features

- [sol-macro] Allow overridden custom errors ([#644](https://github.com/alloy-rs/core/issues/644))

## [0.7.4](https://github.com/alloy-rs/core/releases/tag/v0.7.4) - 2024-05-14

### Bug Fixes

- [sol-macro] Json feature ([#629](https://github.com/alloy-rs/core/issues/629))

## [0.7.3](https://github.com/alloy-rs/core/releases/tag/v0.7.3) - 2024-05-14

### Refactor

- Move `expand` from `sol-macro` to its own crate ([#626](https://github.com/alloy-rs/core/issues/626))

[`dyn-abi`]: https://crates.io/crates/alloy-dyn-abi
[dyn-abi]: https://crates.io/crates/alloy-dyn-abi
[`json-abi`]: https://crates.io/crates/alloy-json-abi
[json-abi]: https://crates.io/crates/alloy-json-abi
[`primitives`]: https://crates.io/crates/alloy-primitives
[primitives]: https://crates.io/crates/alloy-primitives
[`sol-macro`]: https://crates.io/crates/alloy-sol-macro
[sol-macro]: https://crates.io/crates/alloy-sol-macro
[`sol-type-parser`]: https://crates.io/crates/alloy-sol-type-parser
[sol-type-parser]: https://crates.io/crates/alloy-sol-type-parser
[`sol-types`]: https://crates.io/crates/alloy-sol-types
[sol-types]: https://crates.io/crates/alloy-sol-types
[`syn-solidity`]: https://crates.io/crates/syn-solidity
[syn-solidity]: https://crates.io/crates/syn-solidity

<!-- generated by git-cliff -->
