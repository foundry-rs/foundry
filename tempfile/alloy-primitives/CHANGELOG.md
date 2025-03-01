# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.8.20](https://github.com/alloy-rs/core/releases/tag/v0.8.20) - 2025-02-02

### Documentation

- Add 0x to alloy-primitives readme example ([#861](https://github.com/alloy-rs/core/issues/861))

### Features

- Add Sealed::as_sealed_ref ([#859](https://github.com/alloy-rs/core/issues/859))
- Add Sealed::cloned ([#860](https://github.com/alloy-rs/core/issues/860))

### Miscellaneous Tasks

- Release 0.8.20

## [0.8.19](https://github.com/alloy-rs/core/releases/tag/v0.8.19) - 2025-01-15

### Documentation

- Enable some useful rustdoc features on docs.rs ([#850](https://github.com/alloy-rs/core/issues/850))
- Hide hex_literal export ([#849](https://github.com/alloy-rs/core/issues/849))

### Miscellaneous Tasks

- Release 0.8.19

## [0.8.18](https://github.com/alloy-rs/core/releases/tag/v0.8.18) - 2025-01-04

### Bug Fixes

- [primitives] Hex macro re-export ([#848](https://github.com/alloy-rs/core/issues/848))

### Miscellaneous Tasks

- Release 0.8.18

## [0.8.17](https://github.com/alloy-rs/core/releases/tag/v0.8.17) - 2025-01-04

### Features

- Support 0x in hex! and similar macros ([#841](https://github.com/alloy-rs/core/issues/841))
- [primitives] Re-export foldhash ([#839](https://github.com/alloy-rs/core/issues/839))
- Re-export rayon traits implementations ([#836](https://github.com/alloy-rs/core/issues/836))

### Miscellaneous Tasks

- Release 0.8.17

### Testing

- Re-enable miri on foldhash ([#844](https://github.com/alloy-rs/core/issues/844))

## [0.8.16](https://github.com/alloy-rs/core/releases/tag/v0.8.16) - 2025-01-01

### Bug Fixes

- Re-enable foldhash on zkvm ([#833](https://github.com/alloy-rs/core/issues/833))
- Allow non-boolean v values for PrimitiveSignature ([#832](https://github.com/alloy-rs/core/issues/832))

### Features

- Re-export `rayon` feature ([#827](https://github.com/alloy-rs/core/issues/827))

### Miscellaneous Tasks

- Release 0.8.16
- Clippy ([#834](https://github.com/alloy-rs/core/issues/834))
- Add clone_inner ([#825](https://github.com/alloy-rs/core/issues/825))
- Shorten map type alias names ([#824](https://github.com/alloy-rs/core/issues/824))
- [primitives] Remove rustc-hash workaround ([#822](https://github.com/alloy-rs/core/issues/822))

## [0.8.15](https://github.com/alloy-rs/core/releases/tag/v0.8.15) - 2024-12-09

### Miscellaneous Tasks

- Release 0.8.15
- Mark `Signature` as deprecated ([#819](https://github.com/alloy-rs/core/issues/819))
- AsRef for Log ([#820](https://github.com/alloy-rs/core/issues/820))

## [0.8.14](https://github.com/alloy-rs/core/releases/tag/v0.8.14) - 2024-11-28

### Dependencies

- Bump MSRV to 1.81 ([#790](https://github.com/alloy-rs/core/issues/790))

### Features

- Switch all std::error to core::error ([#815](https://github.com/alloy-rs/core/issues/815))

### Miscellaneous Tasks

- Release 0.8.14

## [0.8.13](https://github.com/alloy-rs/core/releases/tag/v0.8.13) - 2024-11-26

### Miscellaneous Tasks

- Release 0.8.13 ([#813](https://github.com/alloy-rs/core/issues/813))

### Other

- Make Signature::new a const fn ([#810](https://github.com/alloy-rs/core/issues/810))

## [0.8.12](https://github.com/alloy-rs/core/releases/tag/v0.8.12) - 2024-11-12

### Bug Fixes

- `Sealed::hash` serde ([#805](https://github.com/alloy-rs/core/issues/805))

### Features

- Add `AsRef` impl and `hash` method to `Sealed` ([#804](https://github.com/alloy-rs/core/issues/804))

### Miscellaneous Tasks

- Release 0.8.12 ([#806](https://github.com/alloy-rs/core/issues/806))

## [0.8.11](https://github.com/alloy-rs/core/releases/tag/v0.8.11) - 2024-11-05

### Bug Fixes

- [serde] Add alias `v` for `yParity` ([#801](https://github.com/alloy-rs/core/issues/801))

### Features

- Add has_eip155_value convenience function to signature ([#791](https://github.com/alloy-rs/core/issues/791))

### Miscellaneous Tasks

- Release 0.8.11 ([#803](https://github.com/alloy-rs/core/issues/803))

### Other

- Revert "chore: replace Signature with PrimitiveSignature" ([#800](https://github.com/alloy-rs/core/issues/800))

### Performance

- Improve normalize_v ([#792](https://github.com/alloy-rs/core/issues/792))

### Styling

- Replace Signature with PrimitiveSignature ([#796](https://github.com/alloy-rs/core/issues/796))

## [0.8.10](https://github.com/alloy-rs/core/releases/tag/v0.8.10) - 2024-10-28

### Bug Fixes

- Revert MSRV changes ([#789](https://github.com/alloy-rs/core/issues/789))

### Dependencies

- Bump MSRV to 1.81 & use `core::error::Error` in place of `std` ([#780](https://github.com/alloy-rs/core/issues/780))

### Miscellaneous Tasks

- Release 0.8.10

### Other

- Implement `DerefMut` for `Log<T>` ([#786](https://github.com/alloy-rs/core/issues/786))

### Refactor

- Use simple boolean for parity in signature ([#776](https://github.com/alloy-rs/core/issues/776))

## [0.8.9](https://github.com/alloy-rs/core/releases/tag/v0.8.9) - 2024-10-21

### Bug Fixes

- Re-enable foldhash by default, but exclude it from zkvm ([#777](https://github.com/alloy-rs/core/issues/777))

### Features

- Expand Seal api ([#773](https://github.com/alloy-rs/core/issues/773))

### Miscellaneous Tasks

- Release 0.8.9

## [0.8.8](https://github.com/alloy-rs/core/releases/tag/v0.8.8) - 2024-10-14

### Bug Fixes

- Properly account for sign in pg to/from sql implementation for signed ([#772](https://github.com/alloy-rs/core/issues/772))
- Don't enable foldhash by default ([#771](https://github.com/alloy-rs/core/issues/771))

### Features

- Add logs_bloom ([#768](https://github.com/alloy-rs/core/issues/768))

### Miscellaneous Tasks

- Release 0.8.8

## [0.8.7](https://github.com/alloy-rs/core/releases/tag/v0.8.7) - 2024-10-08

### Miscellaneous Tasks

- Release 0.8.7

### Other

- Revert "Add custom serialization for Address" ([#765](https://github.com/alloy-rs/core/issues/765))

## [0.8.6](https://github.com/alloy-rs/core/releases/tag/v0.8.6) - 2024-10-06

### Bug Fixes

- Fix lint `alloy-primitives` ([#756](https://github.com/alloy-rs/core/issues/756))

### Dependencies

- [deps] Bump hashbrown to 0.15 ([#753](https://github.com/alloy-rs/core/issues/753))

### Features

- Add `Default` for `Sealed<T>` ([#755](https://github.com/alloy-rs/core/issues/755))
- [primitives] Add and use foldhash as default hasher ([#763](https://github.com/alloy-rs/core/issues/763))

### Miscellaneous Tasks

- Release 0.8.6

### Other

- Derive `Arbitrary` for `Sealed<T>` ([#762](https://github.com/alloy-rs/core/issues/762))
- Derive `Deref` for `Sealed<T>` ([#759](https://github.com/alloy-rs/core/issues/759))
- Add conversion `TxKind` -> `Option<Address>` ([#750](https://github.com/alloy-rs/core/issues/750))

## [0.8.5](https://github.com/alloy-rs/core/releases/tag/v0.8.5) - 2024-09-25

### Bug Fixes

- [primitives] Make sure DefaultHashBuilder implements Clone ([#748](https://github.com/alloy-rs/core/issues/748))

### Miscellaneous Tasks

- Release 0.8.5
- [primitives] Remove Fx* aliases ([#749](https://github.com/alloy-rs/core/issues/749))

## [0.8.4](https://github.com/alloy-rs/core/releases/tag/v0.8.4) - 2024-09-25

### Features

- [primitives] Implement `map` module ([#743](https://github.com/alloy-rs/core/issues/743))
- Support Keccak with sha3 ([#737](https://github.com/alloy-rs/core/issues/737))

### Miscellaneous Tasks

- Release 0.8.4
- Remove unused unstable-doc feature

### Other

- Add custom serialization for Address ([#742](https://github.com/alloy-rs/core/issues/742))

## [0.8.3](https://github.com/alloy-rs/core/releases/tag/v0.8.3) - 2024-09-10

### Features

- Prepare reth Signature migration to alloy ([#732](https://github.com/alloy-rs/core/issues/732))

### Miscellaneous Tasks

- Release 0.8.3

## [0.8.2](https://github.com/alloy-rs/core/releases/tag/v0.8.2) - 2024-09-06

### Bug Fixes

- `no_std` and workflow ([#727](https://github.com/alloy-rs/core/issues/727))

### Documentation

- [primitives] Document features in `wrap_fixed_bytes`-generated types ([#726](https://github.com/alloy-rs/core/issues/726))

### Miscellaneous Tasks

- Release 0.8.2

## [0.8.1](https://github.com/alloy-rs/core/releases/tag/v0.8.1) - 2024-09-06

### Bug Fixes

- Use quantity for v value ([#715](https://github.com/alloy-rs/core/issues/715))

### Dependencies

- Bump MSRV to 1.79 ([#712](https://github.com/alloy-rs/core/issues/712))
- Revert "chore(deps): bump derive_more to 1.0"
- [deps] Bump derive_more to 1.0

### Miscellaneous Tasks

- Release 0.8.1

### Performance

- [primitives] Improve checksum algorithm ([#713](https://github.com/alloy-rs/core/issues/713))

### Refactor

- Remove `Signature` generic ([#719](https://github.com/alloy-rs/core/issues/719))

## [0.8.0](https://github.com/alloy-rs/core/releases/tag/v0.8.0) - 2024-08-21

### Bug Fixes

- Parsing stack overflow ([#703](https://github.com/alloy-rs/core/issues/703))

### Dependencies

- [deps] Bump proptest-derive ([#708](https://github.com/alloy-rs/core/issues/708))

### Documentation

- Typo

### Features

- Derive ser deser on `Sealed` ([#710](https://github.com/alloy-rs/core/issues/710))
- Derive `Hash` for `Sealed` ([#707](https://github.com/alloy-rs/core/issues/707))

### Miscellaneous Tasks

- Release 0.8.0
- [primitives] Re-use ruint mask function ([#698](https://github.com/alloy-rs/core/issues/698))
- Derive hash for parity ([#686](https://github.com/alloy-rs/core/issues/686))

### Other

- Implement specific bit types for integers ([#677](https://github.com/alloy-rs/core/issues/677))
- Add testcase for overflowing_from_sign_and_abs ([#696](https://github.com/alloy-rs/core/issues/696))

### Styling

- Remove `ethereum_ssz` dependency ([#701](https://github.com/alloy-rs/core/issues/701))

## [0.7.7](https://github.com/alloy-rs/core/releases/tag/v0.7.7) - 2024-07-08

### Bug Fixes

- [primitives] Include in aliases export to prevent having to import from `aliases::{..}` ([#655](https://github.com/alloy-rs/core/issues/655))

### Documentation

- [primitives] Fix rustdoc for Signature ([#680](https://github.com/alloy-rs/core/issues/680))
- Add per-crate changelogs ([#669](https://github.com/alloy-rs/core/issues/669))

### Features

- IntoLogData ([#666](https://github.com/alloy-rs/core/issues/666))
- Add `abi_packed_encoded_size` ([#672](https://github.com/alloy-rs/core/issues/672))
- [primitives] Manually implement arbitrary for signature ([#663](https://github.com/alloy-rs/core/issues/663))

### Miscellaneous Tasks

- Release 0.7.7
- Use workspace.lints ([#676](https://github.com/alloy-rs/core/issues/676))

### Styling

- Format some imports
- Sort derives ([#662](https://github.com/alloy-rs/core/issues/662))

## [0.7.6](https://github.com/alloy-rs/core/releases/tag/v0.7.6) - 2024-06-10

### Features

- [primitives] Add additional common aliases ([#654](https://github.com/alloy-rs/core/issues/654))
- [primitives] Derive `Arbitrary` for Signature ([#652](https://github.com/alloy-rs/core/issues/652))
- [primitives] Implement `ops::Not` for fixed bytes ([#650](https://github.com/alloy-rs/core/issues/650))

### Miscellaneous Tasks

- [docs] Add doc aliases for `Tx` prefixed names ([#649](https://github.com/alloy-rs/core/issues/649))

## [0.7.5](https://github.com/alloy-rs/core/releases/tag/v0.7.5) - 2024-06-04

### Bug Fixes

- [primitives] Signed formatting ([#643](https://github.com/alloy-rs/core/issues/643))
- Fix Log serde for non self describing protocols ([#639](https://github.com/alloy-rs/core/issues/639))
- Handle 0 for inverting eip155 parity. ([#633](https://github.com/alloy-rs/core/issues/633))

### Features

- [primitives] Implement TryInto for ParseUnits ([#646](https://github.com/alloy-rs/core/issues/646))

## [0.7.1](https://github.com/alloy-rs/core/releases/tag/v0.7.1) - 2024-04-23

### Features

- Add arbitrary for TxKind ([#604](https://github.com/alloy-rs/core/issues/604))

### Miscellaneous Tasks

- FixedBytes instead of array

## [0.7.0](https://github.com/alloy-rs/core/releases/tag/v0.7.0) - 2024-03-30

### Bug Fixes

- Force clippy to stable ([#569](https://github.com/alloy-rs/core/issues/569))
- [primitives] Re-implement RLP for `Log<LogData>` ([#573](https://github.com/alloy-rs/core/issues/573))

### Documentation

- Do not accept grammar prs ([#575](https://github.com/alloy-rs/core/issues/575))

### Features

- Rlp encoding for logs with generic event data ([#553](https://github.com/alloy-rs/core/issues/553))
- Add LogData::split ([#559](https://github.com/alloy-rs/core/issues/559))

### Miscellaneous Tasks

- No-default-features k256 ([#576](https://github.com/alloy-rs/core/issues/576))

### Other

- Small helpers for alloy serde PR ([#582](https://github.com/alloy-rs/core/issues/582))

### Styling

- Make `Bytes` map to `Bytes` in `SolType` ([#545](https://github.com/alloy-rs/core/issues/545))

## [0.6.4](https://github.com/alloy-rs/core/releases/tag/v0.6.4) - 2024-02-29

### Bug Fixes

- [dyn-abi] Correctly parse empty lists of bytes ([#548](https://github.com/alloy-rs/core/issues/548))

### Documentation

- [primitives] Add a bytes! macro example ([#539](https://github.com/alloy-rs/core/issues/539))

### Features

- Add `TxKind` ([#542](https://github.com/alloy-rs/core/issues/542))
- [core] Re-export `uint!` ([#537](https://github.com/alloy-rs/core/issues/537))
- Derive Allocative on FixedBytes ([#531](https://github.com/alloy-rs/core/issues/531))

### Miscellaneous Tasks

- [primitives] Improve `from_slice` functions ([#546](https://github.com/alloy-rs/core/issues/546))
- Remove unused imports ([#534](https://github.com/alloy-rs/core/issues/534))

## [0.6.3](https://github.com/alloy-rs/core/releases/tag/v0.6.3) - 2024-02-15

### Bug Fixes

- [json-abi] Accept nameless `Param`s ([#526](https://github.com/alloy-rs/core/issues/526))
- Signature bincode serialization ([#509](https://github.com/alloy-rs/core/issues/509))

### Features

- [primitives] Add some more implementations to Bytes ([#528](https://github.com/alloy-rs/core/issues/528))
- Add `alloy-core` prelude crate ([#521](https://github.com/alloy-rs/core/issues/521))
- Make some allocations fallible in ABI decoding ([#513](https://github.com/alloy-rs/core/issues/513))

### Testing

- Remove unused test ([#504](https://github.com/alloy-rs/core/issues/504))

## [0.6.2](https://github.com/alloy-rs/core/releases/tag/v0.6.2) - 2024-01-25

### Bug Fixes

- [`signature`] Construct Signature bytes using v+27 when we do not have an EIP155 `v` ([#503](https://github.com/alloy-rs/core/issues/503))

## [0.6.1](https://github.com/alloy-rs/core/releases/tag/v0.6.1) - 2024-01-25

### Features

- [`primitives`] Add `y_parity_byte_non_eip155` to `Parity` ([#499](https://github.com/alloy-rs/core/issues/499))
- [primitives] Add `Address::from_private_key` ([#483](https://github.com/alloy-rs/core/issues/483))

### Miscellaneous Tasks

- [primitives] Pass B256 by reference in Signature methods ([#487](https://github.com/alloy-rs/core/issues/487))

### Testing

- Parity roundtripping ([#497](https://github.com/alloy-rs/core/issues/497))

## [0.6.0](https://github.com/alloy-rs/core/releases/tag/v0.6.0) - 2024-01-10

### Bug Fixes

- [primitives] Also apply EIP-155 to Parity::Parity ([#476](https://github.com/alloy-rs/core/issues/476))
- Clean the sealed ([#468](https://github.com/alloy-rs/core/issues/468))

### Dependencies

- [deps] Relax k256 requirement ([#481](https://github.com/alloy-rs/core/issues/481))

### Documentation

- Update docs on parity ([#477](https://github.com/alloy-rs/core/issues/477))

### Features

- [primitives] Add Signature type and utils ([#459](https://github.com/alloy-rs/core/issues/459))
- [primitives] Add a buffer type for address checksums ([#472](https://github.com/alloy-rs/core/issues/472))
- [primitives] Add Keccak256 hasher struct ([#469](https://github.com/alloy-rs/core/issues/469))

### Miscellaneous Tasks

- Clippy uninlined_format_args, use_self ([#475](https://github.com/alloy-rs/core/issues/475))

### Refactor

- Log implementation ([#465](https://github.com/alloy-rs/core/issues/465))

## [0.5.4](https://github.com/alloy-rs/core/releases/tag/v0.5.4) - 2023-12-27

### Features

- Sealed ([#467](https://github.com/alloy-rs/core/issues/467))
- [primitives] Re-export ::bytes ([#462](https://github.com/alloy-rs/core/issues/462))
- [primitives] Support parsing numbers in Unit::from_str ([#461](https://github.com/alloy-rs/core/issues/461))
- Enable postgres ruint feature ([#460](https://github.com/alloy-rs/core/issues/460))

### Miscellaneous Tasks

- Clean up address checksum implementation ([#464](https://github.com/alloy-rs/core/issues/464))

### Performance

- Add optional support for keccak-asm ([#466](https://github.com/alloy-rs/core/issues/466))

### Styling

- Add ToSql and FromSql to Signed and FixedBytes ([#447](https://github.com/alloy-rs/core/issues/447))

## [0.5.3](https://github.com/alloy-rs/core/releases/tag/v0.5.3) - 2023-12-16

### Bug Fixes

- [primitives] Return correct fixed length in ssz::Encode ([#451](https://github.com/alloy-rs/core/issues/451))

### Features

- Address from pubkey ([#455](https://github.com/alloy-rs/core/issues/455))
- [primitives] Update Bytes formatting, add UpperHex ([#446](https://github.com/alloy-rs/core/issues/446))

## [0.5.0](https://github.com/alloy-rs/core/releases/tag/v0.5.0) - 2023-11-23

### Bug Fixes

- Avoid symlinks ([#396](https://github.com/alloy-rs/core/issues/396))
- [primitives] Signed cleanup ([#395](https://github.com/alloy-rs/core/issues/395))

### Features

- [primitives] Left and right padding conversions ([#424](https://github.com/alloy-rs/core/issues/424))
- [primitives] Improve utils ([#432](https://github.com/alloy-rs/core/issues/432))
- [sol-macro] `SolEventInterface`: `SolInterface` for contract events enum ([#426](https://github.com/alloy-rs/core/issues/426))
- Enable ruint ssz when primitives ssz ([#419](https://github.com/alloy-rs/core/issues/419))
- [dyn-abi] `DynSolType::coerce_str` ([#380](https://github.com/alloy-rs/core/issues/380))

### Miscellaneous Tasks

- Clean up ABI, EIP-712, docs ([#373](https://github.com/alloy-rs/core/issues/373))

### Other

- SSZ implementation for alloy primitives ([#407](https://github.com/alloy-rs/core/issues/407))
- Enable rand feature for re-exported ruint crate ([#385](https://github.com/alloy-rs/core/issues/385))

### Styling

- Update rustfmt config ([#406](https://github.com/alloy-rs/core/issues/406))

## [0.4.2](https://github.com/alloy-rs/core/releases/tag/v0.4.2) - 2023-10-09

### Bug Fixes

- [primitives] Set serde derive feature ([#359](https://github.com/alloy-rs/core/issues/359))

## [0.4.1](https://github.com/alloy-rs/core/releases/tag/v0.4.1) - 2023-10-09

### Features

- Add parsing support for JSON items ([#329](https://github.com/alloy-rs/core/issues/329))
- Add logs, add log dynamic decoding ([#271](https://github.com/alloy-rs/core/issues/271))

### Miscellaneous Tasks

- Enable ruint std feature ([#326](https://github.com/alloy-rs/core/issues/326))

### Other

- Run miri in ci ([#327](https://github.com/alloy-rs/core/issues/327))

## [0.4.0](https://github.com/alloy-rs/core/releases/tag/v0.4.0) - 2023-09-29

### Bug Fixes

- Add super import on generated modules ([#307](https://github.com/alloy-rs/core/issues/307))
- Hex compatibility ([#244](https://github.com/alloy-rs/core/issues/244))

### Documentation

- Improve `ResolveSolType` documentation ([#296](https://github.com/alloy-rs/core/issues/296))
- Add note regarding ruint::uint macro ([#265](https://github.com/alloy-rs/core/issues/265))
- Update fixed bytes docs ([#255](https://github.com/alloy-rs/core/issues/255))

### Features

- [json-abi] Add `Function::signature_full` ([#289](https://github.com/alloy-rs/core/issues/289))
- [primitives] Add more methods to `Function` ([#290](https://github.com/alloy-rs/core/issues/290))
- Add more `FixedBytes` to int conversion impls ([#281](https://github.com/alloy-rs/core/issues/281))
- Add support for `rand` ([#282](https://github.com/alloy-rs/core/issues/282))
- Impl `bytes::Buf` for our own `Bytes` ([#279](https://github.com/alloy-rs/core/issues/279))
- Add more `Bytes` conversion impls ([#280](https://github.com/alloy-rs/core/issues/280))
- [primitives] Improve Bytes ([#269](https://github.com/alloy-rs/core/issues/269))
- [primitives] Allow empty input in hex macros ([#245](https://github.com/alloy-rs/core/issues/245))

### Miscellaneous Tasks

- Sync crate level attributes ([#303](https://github.com/alloy-rs/core/issues/303))
- Use `hex!` macro from `primitives` re-export ([#299](https://github.com/alloy-rs/core/issues/299))
- Re-export ::bytes ([#278](https://github.com/alloy-rs/core/issues/278))

### Other

- Hash_message ([#304](https://github.com/alloy-rs/core/issues/304))
- Typo ([#249](https://github.com/alloy-rs/core/issues/249))

### Performance

- Use `slice::Iter` where possible ([#256](https://github.com/alloy-rs/core/issues/256))

### Styling

- Some clippy lints ([#251](https://github.com/alloy-rs/core/issues/251))

## [0.3.2](https://github.com/alloy-rs/core/releases/tag/v0.3.2) - 2023-08-23

### Bug Fixes

- Fix bincode serialization ([#223](https://github.com/alloy-rs/core/issues/223))

### Features

- [primitives] More `FixedBytes<N>` <-> `[u8; N]` conversions ([#239](https://github.com/alloy-rs/core/issues/239))
- Function type ([#224](https://github.com/alloy-rs/core/issues/224))

### Miscellaneous Tasks

- [primitives] Discourage use of `B160` ([#235](https://github.com/alloy-rs/core/issues/235))
- Clippy ([#225](https://github.com/alloy-rs/core/issues/225))

### Performance

- Optimize some stuff ([#231](https://github.com/alloy-rs/core/issues/231))

## [0.3.0](https://github.com/alloy-rs/core/releases/tag/v0.3.0) - 2023-07-26

### Bug Fixes

- [alloy-primitives] Fix broken documentation link ([#152](https://github.com/alloy-rs/core/issues/152))

### Features

- Bytes handles numeric arrays and bytearrays in deser ([#202](https://github.com/alloy-rs/core/issues/202))
- Native keccak feature flag ([#185](https://github.com/alloy-rs/core/issues/185))
- [rlp] Improve implementations ([#182](https://github.com/alloy-rs/core/issues/182))
- [dyn-abi] Add arbitrary impls and proptests ([#175](https://github.com/alloy-rs/core/issues/175))
- [dyn-abi] Clean up and improve performance ([#174](https://github.com/alloy-rs/core/issues/174))
- [json-abi] Add more impls ([#164](https://github.com/alloy-rs/core/issues/164))
- [primitives] Add some impls ([#162](https://github.com/alloy-rs/core/issues/162))
- `SolEnum` and `SolInterface` ([#153](https://github.com/alloy-rs/core/issues/153))
- [primitives] Fixed bytes macros ([#156](https://github.com/alloy-rs/core/issues/156))

### Miscellaneous Tasks

- Wrap Bytes methods which return Self ([#206](https://github.com/alloy-rs/core/issues/206))
- Warn on all rustdoc lints ([#154](https://github.com/alloy-rs/core/issues/154))
- Add smaller image for favicon ([#142](https://github.com/alloy-rs/core/issues/142))

### Other

- Kuly14/cleanup ([#151](https://github.com/alloy-rs/core/issues/151))

## [0.2.0](https://github.com/alloy-rs/core/releases/tag/v0.2.0) - 2023-06-23

### Bug Fixes

- (u)int tokenization ([#123](https://github.com/alloy-rs/core/issues/123))
- Rlp impls ([#56](https://github.com/alloy-rs/core/issues/56))
- Hex breaking change ([#50](https://github.com/alloy-rs/core/issues/50))

### Dependencies

- Bump ruint to have alloy-rlp

### Features

- More FixedBytes impls ([#111](https://github.com/alloy-rs/core/issues/111))
- Primitive utils and improvements ([#52](https://github.com/alloy-rs/core/issues/52))

### Miscellaneous Tasks

- Add logo to all crates, add @gakonst to CODEOWNERS ([#138](https://github.com/alloy-rs/core/issues/138))
- Clean up features ([#116](https://github.com/alloy-rs/core/issues/116))
- Feature-gate `getrandom`, document in README.md ([#71](https://github.com/alloy-rs/core/issues/71))
- Rename to Alloy ([#69](https://github.com/alloy-rs/core/issues/69))
- Enable `feature(doc_cfg, doc_auto_cfg)` ([#67](https://github.com/alloy-rs/core/issues/67))
- Pre-release mega cleanup ([#35](https://github.com/alloy-rs/core/issues/35))
- Use crates.io uint, move crates to `crates/*` ([#31](https://github.com/alloy-rs/core/issues/31))

### Other

- Fix dep job, add feature-checks job ([#64](https://github.com/alloy-rs/core/issues/64))
- Prestwich/crate readmes ([#41](https://github.com/alloy-rs/core/issues/41))

### Performance

- Improve rlp, update Address methods ([#118](https://github.com/alloy-rs/core/issues/118))

### Refactor

- Implement `SolType` for `{Ui,I}nt<N>` and `FixedBytes<N>` with const-generics ([#92](https://github.com/alloy-rs/core/issues/92))

### Styling

- Add rustfmt.toml ([#42](https://github.com/alloy-rs/core/issues/42))

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
