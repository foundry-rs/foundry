# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.7.9](https://github.com/alloy-rs/trie/releases/tag/v0.7.9) - 2025-02-07

### Features

- Add DecodedProofNodes struct ([#81](https://github.com/alloy-rs/trie/issues/81))

## [0.7.8](https://github.com/alloy-rs/trie/releases/tag/v0.7.8) - 2024-12-31

### Dependencies

- Bump and use nybbles 0.3.3 raw APIs ([#80](https://github.com/alloy-rs/trie/issues/80))

### Miscellaneous Tasks

- Add tests for `TrieAccount` ([#73](https://github.com/alloy-rs/trie/issues/73))

## [0.7.7](https://github.com/alloy-rs/trie/releases/tag/v0.7.7) - 2024-12-22

### Features

- Bump nybbles, use local encode_path ([#76](https://github.com/alloy-rs/trie/issues/76))

### Miscellaneous Tasks

- Release 0.7.7
- Use-single-account-buf ([#78](https://github.com/alloy-rs/trie/issues/78))

### Other

- Move deny to ci ([#75](https://github.com/alloy-rs/trie/issues/75))

## [0.7.6](https://github.com/alloy-rs/trie/releases/tag/v0.7.6) - 2024-12-04

### Features

- Add storage root fns ([#74](https://github.com/alloy-rs/trie/issues/74))

### Miscellaneous Tasks

- Release 0.7.6

## [0.7.5](https://github.com/alloy-rs/trie/releases/tag/v0.7.5) - 2024-12-04

### Dependencies

- Bump MSRV to 1.81 ([#66](https://github.com/alloy-rs/trie/issues/66))

### Documentation

- Clarify the documentation for `hash_mask` field of a branch node ([#70](https://github.com/alloy-rs/trie/issues/70))

### Features

- Migrate trie account type and state root functions from alloy ([#65](https://github.com/alloy-rs/trie/issues/65))
- `HashBuilder::add_leaf_unchecked` ([#64](https://github.com/alloy-rs/trie/issues/64))
- Derive `Clone` for `HashBuilder` ([#72](https://github.com/alloy-rs/trie/issues/72))

### Miscellaneous Tasks

- Release 0.7.5
- Add clippy settings to `Cargo.toml` ([#71](https://github.com/alloy-rs/trie/issues/71))
- Update cargo deny ([#69](https://github.com/alloy-rs/trie/issues/69))

## [0.7.4](https://github.com/alloy-rs/trie/releases/tag/v0.7.4) - 2024-11-13

### Features

- Impl Extend for ProofNodes ([#63](https://github.com/alloy-rs/trie/issues/63))

### Miscellaneous Tasks

- Release 0.7.4

## [0.7.3](https://github.com/alloy-rs/trie/releases/tag/v0.7.3) - 2024-11-07

### Documentation

- [nodes] Adjust comments about branch node masks ([#61](https://github.com/alloy-rs/trie/issues/61))

### Features

- [nodes] Make `BranchNodeRef::children` public ([#62](https://github.com/alloy-rs/trie/issues/62))

### Miscellaneous Tasks

- Release 0.7.3
- [hash-builder] Use `RlpNode::as_hash` ([#59](https://github.com/alloy-rs/trie/issues/59))

### Styling

- Migrated functions for computing trie root from reth to alloy ([#55](https://github.com/alloy-rs/trie/issues/55))

## [0.7.2](https://github.com/alloy-rs/trie/releases/tag/v0.7.2) - 2024-10-16

### Features

- [mask] Unset bit, count set bits, index of the first set bit ([#58](https://github.com/alloy-rs/trie/issues/58))
- `RlpNode::as_hash` ([#57](https://github.com/alloy-rs/trie/issues/57))

### Miscellaneous Tasks

- Release 0.7.2

## [0.7.1](https://github.com/alloy-rs/trie/releases/tag/v0.7.1) - 2024-10-14

### Bug Fixes

- Use vector of arbitrary length for leaf node value ([#56](https://github.com/alloy-rs/trie/issues/56))

### Miscellaneous Tasks

- Release 0.7.1
- Allow `Zlib` in `deny.toml` ([#54](https://github.com/alloy-rs/trie/issues/54))

## [0.7.0](https://github.com/alloy-rs/trie/releases/tag/v0.7.0) - 2024-10-14

### Bug Fixes

- Arbitrary impls ([#52](https://github.com/alloy-rs/trie/issues/52))

### Miscellaneous Tasks

- Release 0.7.0
- [meta] Add CODEOWNERS ([#47](https://github.com/alloy-rs/trie/issues/47))

### Performance

- Avoid cloning HashBuilder input ([#50](https://github.com/alloy-rs/trie/issues/50))
- Store RLP-encoded nodes using ArrayVec ([#51](https://github.com/alloy-rs/trie/issues/51))
- Avoid calculating branch node children if possible ([#49](https://github.com/alloy-rs/trie/issues/49))
- Inline RLP encode functions ([#46](https://github.com/alloy-rs/trie/issues/46))

## [0.6.0](https://github.com/alloy-rs/trie/releases/tag/v0.6.0) - 2024-09-26

### Features

- Replace std/hashbrown with alloy_primitives::map ([#42](https://github.com/alloy-rs/trie/issues/42))
- Empty root node ([#36](https://github.com/alloy-rs/trie/issues/36))

### Miscellaneous Tasks

- Release 0.6.0
- Display more information on assertions ([#40](https://github.com/alloy-rs/trie/issues/40))
- Expose `rlp_node` ([#38](https://github.com/alloy-rs/trie/issues/38))
- Remove children hashes methods ([#35](https://github.com/alloy-rs/trie/issues/35))

### Performance

- Change proof internal repr to `HashMap` ([#43](https://github.com/alloy-rs/trie/issues/43))
- [proof] Compare slices for first node ([#37](https://github.com/alloy-rs/trie/issues/37))

## [0.5.3](https://github.com/alloy-rs/trie/releases/tag/v0.5.3) - 2024-09-17

### Dependencies

- Bump msrv to 1.79 ([#33](https://github.com/alloy-rs/trie/issues/33))

### Miscellaneous Tasks

- Release 0.5.3
- Release 0.5.2
- Use `decode_raw` from `alloy-rlp` ([#19](https://github.com/alloy-rs/trie/issues/19))

### Testing

- Zero value leaf ([#34](https://github.com/alloy-rs/trie/issues/34))

## [0.5.1](https://github.com/alloy-rs/trie/releases/tag/v0.5.1) - 2024-09-02

### Bug Fixes

- No-std compat ([#31](https://github.com/alloy-rs/trie/issues/31))

### Features

- Workflow to validate no_std compatibility ([#32](https://github.com/alloy-rs/trie/issues/32))

### Miscellaneous Tasks

- Release 0.5.1

## [0.5.0](https://github.com/alloy-rs/trie/releases/tag/v0.5.0) - 2024-08-28

### Bug Fixes

- In-place nodes ignored in proof verification ([#27](https://github.com/alloy-rs/trie/issues/27))

### Dependencies

- Bump derive more ([#30](https://github.com/alloy-rs/trie/issues/30))
- [deps] Bump alloy ([#28](https://github.com/alloy-rs/trie/issues/28))
- Bump proptest ([#18](https://github.com/alloy-rs/trie/issues/18))

### Documentation

- Small fix on HashBuilderValue  docs ([#20](https://github.com/alloy-rs/trie/issues/20))

### Features

- Iterator over branch children ([#21](https://github.com/alloy-rs/trie/issues/21))

### Miscellaneous Tasks

- Release 0.5.0
- Make clippy happy ([#29](https://github.com/alloy-rs/trie/issues/29))
- Make `TrieNode` cloneable ([#22](https://github.com/alloy-rs/trie/issues/22))
- Make clippy happy ([#17](https://github.com/alloy-rs/trie/issues/17))
- Sync cliff.toml

## [0.4.1](https://github.com/alloy-rs/trie/releases/tag/v0.4.1) - 2024-05-22

### Bug Fixes

- Proofs for divergent leaf nodes ([#16](https://github.com/alloy-rs/trie/issues/16))

### Dependencies

- Move path encoding from `nybbles` ([#14](https://github.com/alloy-rs/trie/issues/14))

### Miscellaneous Tasks

- Release 0.4.1

## [0.4.0](https://github.com/alloy-rs/trie/releases/tag/v0.4.0) - 2024-05-14

### Features

- Proof verification ([#13](https://github.com/alloy-rs/trie/issues/13))
- Branch node decoding ([#12](https://github.com/alloy-rs/trie/issues/12))
- Extension node decoding ([#11](https://github.com/alloy-rs/trie/issues/11))
- Leaf node decoding ([#10](https://github.com/alloy-rs/trie/issues/10))

### Miscellaneous Tasks

- Release 0.4.0

## [0.3.1](https://github.com/alloy-rs/trie/releases/tag/v0.3.1) - 2024-04-03

### Dependencies

- Bump alloy-primitives 0.7.0 ([#8](https://github.com/alloy-rs/trie/issues/8))

### Miscellaneous Tasks

- Release 0.3.1
- Fix loop span ([#6](https://github.com/alloy-rs/trie/issues/6))

## [0.3.0](https://github.com/alloy-rs/trie/releases/tag/v0.3.0) - 2024-02-26

### Dependencies

- [deps] Bump nybbles to 0.2 ([#4](https://github.com/alloy-rs/trie/issues/4))

### Miscellaneous Tasks

- Release 0.3.0
- Clippy ([#5](https://github.com/alloy-rs/trie/issues/5))

## [0.2.1](https://github.com/alloy-rs/trie/releases/tag/v0.2.1) - 2024-01-24

### Features

- Support no_std ([#2](https://github.com/alloy-rs/trie/issues/2))

### Miscellaneous Tasks

- Release 0.2.1
- Add cliff.toml and scripts
- Simplify no_std

## [0.2.0](https://github.com/alloy-rs/trie/releases/tag/v0.2.0) - 2024-01-10

### Dependencies

- [deps] Bump alloy-primitives

### Miscellaneous Tasks

- Release 0.2.0
- Update authors ([#3](https://github.com/alloy-rs/trie/issues/3))

## [0.1.0](https://github.com/alloy-rs/trie/releases/tag/v0.1.0) - 2023-12-20

### Bug Fixes

- Miri
- Deny

### Dependencies

- Clean up dependencies

### Features

- Initial implementation extracted from reth

### Miscellaneous Tasks

- Update configs
- Increase visibility
- Remove unused module
- Clippy, docs, rm unused file
- [meta] Ci, licenses, configs

### Other

- Prealloc children ([#1](https://github.com/alloy-rs/trie/issues/1))
- Initial commit

<!-- generated by git-cliff -->
