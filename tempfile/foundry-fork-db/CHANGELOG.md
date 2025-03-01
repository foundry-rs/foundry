# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.11.0](https://github.com/foundry-rs/foundry-fork-db/releases/tag/v0.11.0) - 2025-01-31

### Dependencies

- Bump alloy 0.11 ([#41](https://github.com/foundry-rs/foundry-fork-db/issues/41))

## [0.10.0](https://github.com/foundry-rs/foundry-fork-db/releases/tag/v0.10.0) - 2024-12-30

### Features

- Update revm 19 alloy 09 ([#39](https://github.com/foundry-rs/foundry-fork-db/issues/39))

### Miscellaneous Tasks

- Release 0.10.0

## [0.9.0](https://github.com/foundry-rs/foundry-fork-db/releases/tag/v0.9.0) - 2024-12-10

### Dependencies

- Bump alloy 0.8 ([#38](https://github.com/foundry-rs/foundry-fork-db/issues/38))
- Bump MSRV to 1.81 ([#37](https://github.com/foundry-rs/foundry-fork-db/issues/37))
- Bump breaking deps ([#36](https://github.com/foundry-rs/foundry-fork-db/issues/36))

### Miscellaneous Tasks

- Release 0.9.0
- Update deny.toml ([#35](https://github.com/foundry-rs/foundry-fork-db/issues/35))

### Other

- Move deny to ci ([#34](https://github.com/foundry-rs/foundry-fork-db/issues/34))

## [0.8.0](https://github.com/foundry-rs/foundry-fork-db/releases/tag/v0.8.0) - 2024-11-28

### Dependencies

- Bump alloy ([#33](https://github.com/foundry-rs/foundry-fork-db/issues/33))

### Miscellaneous Tasks

- Release 0.8.0

## [0.7.2](https://github.com/foundry-rs/foundry-fork-db/releases/tag/v0.7.2) - 2024-11-27

### Documentation

- Fix typo in changelog generator 2
- Fix typo in changelog generator

### Features

- [backend] Add support for arbitrary provider requests with AnyRequest ([#32](https://github.com/foundry-rs/foundry-fork-db/issues/32))

### Miscellaneous Tasks

- Release 0.7.2

## [0.7.1](https://github.com/foundry-rs/foundry-fork-db/releases/tag/v0.7.1) - 2024-11-09

### Bug Fixes

- Accept generic header in meta builder ([#30](https://github.com/foundry-rs/foundry-fork-db/issues/30))

### Miscellaneous Tasks

- Release 0.7.1

## [0.7.0](https://github.com/foundry-rs/foundry-fork-db/releases/tag/v0.7.0) - 2024-11-08

### Dependencies

- [deps] Bump alloy 0.6.2 ([#29](https://github.com/foundry-rs/foundry-fork-db/issues/29))

### Documentation

- Update docs

### Miscellaneous Tasks

- Release 0.7.0

## [0.6.0](https://github.com/foundry-rs/foundry-fork-db/releases/tag/v0.6.0) - 2024-10-23

### Dependencies

- Bump revm ([#27](https://github.com/foundry-rs/foundry-fork-db/issues/27))

### Miscellaneous Tasks

- Release 0.6.0

## [0.5.0](https://github.com/foundry-rs/foundry-fork-db/releases/tag/v0.5.0) - 2024-10-18

### Dependencies

- Bump alloy 0.5 ([#26](https://github.com/foundry-rs/foundry-fork-db/issues/26))

### Miscellaneous Tasks

- Release 0.5.0

## [0.4.0](https://github.com/foundry-rs/foundry-fork-db/releases/tag/v0.4.0) - 2024-09-30

### Dependencies

- Bump alloy 0.4 ([#24](https://github.com/foundry-rs/foundry-fork-db/issues/24))

### Miscellaneous Tasks

- Release 0.4.0

## [0.3.2](https://github.com/foundry-rs/foundry-fork-db/releases/tag/v0.3.2) - 2024-09-29

### Features

- BlockchainDbMeta builder ([#22](https://github.com/foundry-rs/foundry-fork-db/issues/22))

### Miscellaneous Tasks

- Release 0.3.2
- Use more alloy_primitives::map

## [0.3.1](https://github.com/foundry-rs/foundry-fork-db/releases/tag/v0.3.1) - 2024-09-21

### Dependencies

- [deps] Disable default features for revm ([#20](https://github.com/foundry-rs/foundry-fork-db/issues/20))

### Miscellaneous Tasks

- Release 0.3.1

### Other

- Don't deploy docs

## [0.3.0](https://github.com/foundry-rs/foundry-fork-db/releases/tag/v0.3.0) - 2024-08-29

### Bug Fixes

- Fix fmt

### Dependencies

- Merge pull request [#19](https://github.com/foundry-rs/foundry-fork-db/issues/19) from foundry-rs/matt/bump-alloy03
- Bump alloy

### Miscellaneous Tasks

- Release 0.3.0

### Other

- Update
- Merge pull request [#18](https://github.com/foundry-rs/foundry-fork-db/issues/18) from nkysg/unbound_channel
- Rm clone
- Replace bounded channel with unbounded channel

## [0.2.1](https://github.com/foundry-rs/foundry-fork-db/releases/tag/v0.2.1) - 2024-08-08

### Bug Fixes

- Fix clippy
- Fix-tests after checking

### Dependencies

- Merge pull request [#17](https://github.com/foundry-rs/foundry-fork-db/issues/17) from foundry-rs/matt/bump-revm13
- Bump revm 13
- Undo bump version
- Bump version of crate
- Merge bump-revm

### Documentation

- Docs to functions
- Docs

### Miscellaneous Tasks

- Release 0.2.1

### Other

- Merge pull request [#16](https://github.com/foundry-rs/foundry-fork-db/issues/16) from m1stoyanov/patch-1
- Remove the unnecessary result from the helper functions
- Provide helper methods for MemDb data
- Merge pull request [#13](https://github.com/foundry-rs/foundry-fork-db/issues/13) from nkysg/sharedbackend_behaviour
- Update process logic
- Add BlockingMod::Block process
-  add configure for SharedBackend block_in_place or not
- Merge pull request [#10](https://github.com/foundry-rs/foundry-fork-db/issues/10) from Ethanol48/update_state
- Eliminated tmp ETH_RPC
- Added tmp file for testing
- Eliminate reduntant code
- Add tests to verify if the data was properly updated
- Added db to test to verify data
- Add minor changes
- Update block hashes
- Typo
- Update address in db
- Update revm
- Merge pull request [#12](https://github.com/foundry-rs/foundry-fork-db/issues/12) from Ethanol48/flush_to_file
- Change to &Path
- Eliminate reduntant code
- Merge branch 'main' of https://github.com/Ethanol48/foundry-fork-db into flush_to_file

### Refactor

- Refactor and storage update
- Refactoring

## [0.2.0](https://github.com/foundry-rs/foundry-fork-db/releases/tag/v0.2.0) - 2024-07-17

### Dependencies

- Merge pull request [#8](https://github.com/foundry-rs/foundry-fork-db/issues/8) from foundry-rs/klkvr/bump-revm
- Bump revm
- Merge pull request [#7](https://github.com/foundry-rs/foundry-fork-db/issues/7) from foundry-rs/matt/bump-revm-alloy
- Bump alloy and revm

### Miscellaneous Tasks

- Release 0.2.0

### Other

- Formating
- Add documentation
- Add flush to arbitrary file

## [0.1.1](https://github.com/foundry-rs/foundry-fork-db/releases/tag/v0.1.1) - 2024-07-15

### Dependencies

- Merge pull request [#5](https://github.com/foundry-rs/foundry-fork-db/issues/5) from foundry-rs/matt/bump-msrv
- Bump msrv 79
- Merge pull request [#4](https://github.com/foundry-rs/foundry-fork-db/issues/4) from m1stoyanov/main
- Bump alloy [provider, rpc-types, serde, transport, rpc-client, transport-http] to 0.1.4, alloy-primitives to 0.7.7 and revm to 11.0.0

### Miscellaneous Tasks

- Release 0.1.1

### Other

- Remove redundant check
- Update Cargo.toml according to the reviews

## [0.1.0](https://github.com/foundry-rs/foundry-fork-db/releases/tag/v0.1.0) - 2024-07-02

### Bug Fixes

- Clippy
- Cargo deny
- Clippy + fmt
- Tests

### Miscellaneous Tasks

- Release 0.1.0
- Init changelog
- Fix cliff.toml
- Add description

### Other

- Update naming ([#2](https://github.com/foundry-rs/foundry-fork-db/issues/2))
- Merge pull request [#1](https://github.com/foundry-rs/foundry-fork-db/issues/1) from klkvr/klkvr/init
- DatabaseError -> BackendError
- Initial commit
- Update readme
- Update name
- Initial commit

<!-- generated by git-cliff -->
