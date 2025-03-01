# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.8.21](https://github.com/alloy-rs/core/releases/tag/v0.8.21) - 2025-02-10

### Bug Fixes

- [sol-macro] Call proc_macro_error handler manually ([#866](https://github.com/alloy-rs/core/issues/866))

### Features

- Add helpers for revertreason ([#867](https://github.com/alloy-rs/core/issues/867))

## [0.8.20](https://github.com/alloy-rs/core/releases/tag/v0.8.20) - 2025-02-02

### Documentation

- Add 0x to alloy-primitives readme example ([#861](https://github.com/alloy-rs/core/issues/861))

### Miscellaneous Tasks

- Release 0.8.20

## [0.8.19](https://github.com/alloy-rs/core/releases/tag/v0.8.19) - 2025-01-15

### Documentation

- Enable some useful rustdoc features on docs.rs ([#850](https://github.com/alloy-rs/core/issues/850))

### Features

- [sol-types] Improve ABI decoding error messages ([#851](https://github.com/alloy-rs/core/issues/851))

### Miscellaneous Tasks

- Release 0.8.19

## [0.8.18](https://github.com/alloy-rs/core/releases/tag/v0.8.18) - 2025-01-04

### Miscellaneous Tasks

- Release 0.8.18

## [0.8.17](https://github.com/alloy-rs/core/releases/tag/v0.8.17) - 2025-01-04

### Documentation

- Typos ([#847](https://github.com/alloy-rs/core/issues/847))

### Features

- [sol-macro] Translate contract types to address ([#842](https://github.com/alloy-rs/core/issues/842))
- Support 0x in hex! and similar macros ([#841](https://github.com/alloy-rs/core/issues/841))
- [sol-macro] Evaluate array sizes ([#840](https://github.com/alloy-rs/core/issues/840))

### Miscellaneous Tasks

- Release 0.8.17

### Testing

- [sol-macro] Add a test for namespaced types ([#843](https://github.com/alloy-rs/core/issues/843))

## [0.8.16](https://github.com/alloy-rs/core/releases/tag/v0.8.16) - 2025-01-01

### Bug Fixes

- [syn-solidity] Correctly parse invalid bytes* etc as custom ([#830](https://github.com/alloy-rs/core/issues/830))

### Miscellaneous Tasks

- Release 0.8.16

## [0.8.15](https://github.com/alloy-rs/core/releases/tag/v0.8.15) - 2024-12-09

### Miscellaneous Tasks

- Release 0.8.15

## [0.8.14](https://github.com/alloy-rs/core/releases/tag/v0.8.14) - 2024-11-28

### Dependencies

- Bump MSRV to 1.81 ([#790](https://github.com/alloy-rs/core/issues/790))

### Features

- Switch all std::error to core::error ([#815](https://github.com/alloy-rs/core/issues/815))

### Miscellaneous Tasks

- Release 0.8.14

## [0.8.13](https://github.com/alloy-rs/core/releases/tag/v0.8.13) - 2024-11-26

### Bug Fixes

- [sol-macro] Expand all getter return types ([#812](https://github.com/alloy-rs/core/issues/812))

### Miscellaneous Tasks

- Release 0.8.13 ([#813](https://github.com/alloy-rs/core/issues/813))

## [0.8.12](https://github.com/alloy-rs/core/releases/tag/v0.8.12) - 2024-11-12

### Miscellaneous Tasks

- Release 0.8.12 ([#806](https://github.com/alloy-rs/core/issues/806))

## [0.8.11](https://github.com/alloy-rs/core/releases/tag/v0.8.11) - 2024-11-05

### Miscellaneous Tasks

- Release 0.8.11 ([#803](https://github.com/alloy-rs/core/issues/803))

## [0.8.10](https://github.com/alloy-rs/core/releases/tag/v0.8.10) - 2024-10-28

### Bug Fixes

- Revert MSRV changes ([#789](https://github.com/alloy-rs/core/issues/789))

### Dependencies

- Bump MSRV to 1.81 & use `core::error::Error` in place of `std` ([#780](https://github.com/alloy-rs/core/issues/780))

### Miscellaneous Tasks

- Release 0.8.10
- Address MSRV TODOs for 1.81 ([#781](https://github.com/alloy-rs/core/issues/781))

## [0.8.9](https://github.com/alloy-rs/core/releases/tag/v0.8.9) - 2024-10-21

### Miscellaneous Tasks

- Release 0.8.9

## [0.8.8](https://github.com/alloy-rs/core/releases/tag/v0.8.8) - 2024-10-14

### Miscellaneous Tasks

- Release 0.8.8

## [0.8.7](https://github.com/alloy-rs/core/releases/tag/v0.8.7) - 2024-10-08

### Miscellaneous Tasks

- Release 0.8.7

## [0.8.6](https://github.com/alloy-rs/core/releases/tag/v0.8.6) - 2024-10-06

### Bug Fixes

- Fix lint alloy-sol-types ([#761](https://github.com/alloy-rs/core/issues/761))

### Miscellaneous Tasks

- Release 0.8.6
- Remove a stabilized impl_core function

## [0.8.5](https://github.com/alloy-rs/core/releases/tag/v0.8.5) - 2024-09-25

### Miscellaneous Tasks

- Release 0.8.5

## [0.8.4](https://github.com/alloy-rs/core/releases/tag/v0.8.4) - 2024-09-25

### Bug Fixes

- [json-abi] Normalize $ to _ in identifiers in to_sol ([#747](https://github.com/alloy-rs/core/issues/747))
- [json-abi] Correct to-sol for UDVT arrays in structs ([#745](https://github.com/alloy-rs/core/issues/745))
- [sol-types] Check signature in SolEvent if non-anonymous ([#741](https://github.com/alloy-rs/core/issues/741))

### Miscellaneous Tasks

- Release 0.8.4

### Testing

- Allow missing_docs in tests

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
- Revert "chore(deps): bump derive_more to 1.0"
- [deps] Bump derive_more to 1.0

### Miscellaneous Tasks

- Release 0.8.1
- Clippy

### Testing

- [sol] Add a test for custom paths

## [0.8.0](https://github.com/alloy-rs/core/releases/tag/v0.8.0) - 2024-08-21

### Dependencies

- [deps] Bump proptest-derive ([#708](https://github.com/alloy-rs/core/issues/708))

### Features

- [sol-macro] Support namespaces ([#694](https://github.com/alloy-rs/core/issues/694))
- [sol-types] Implement traits for longer tuples ([#699](https://github.com/alloy-rs/core/issues/699))

### Miscellaneous Tasks

- Release 0.8.0
- Add some TODO comments

### Other

- Implement specific bit types for integers ([#677](https://github.com/alloy-rs/core/issues/677))

## [0.7.7](https://github.com/alloy-rs/core/releases/tag/v0.7.7) - 2024-07-08

### Documentation

- [sol-types] Update README.md using crate docs ([#679](https://github.com/alloy-rs/core/issues/679))
- Add per-crate changelogs ([#669](https://github.com/alloy-rs/core/issues/669))

### Features

- IntoLogData ([#666](https://github.com/alloy-rs/core/issues/666))
- Add `abi_packed_encoded_size` ([#672](https://github.com/alloy-rs/core/issues/672))

### Miscellaneous Tasks

- Release 0.7.7
- Use workspace.lints ([#676](https://github.com/alloy-rs/core/issues/676))
- Fix unnameable-types ([#675](https://github.com/alloy-rs/core/issues/675))
- Swap sol macro doctests symlink ([#657](https://github.com/alloy-rs/core/issues/657))

### Styling

- Sort derives ([#662](https://github.com/alloy-rs/core/issues/662))

## [0.7.6](https://github.com/alloy-rs/core/releases/tag/v0.7.6) - 2024-06-10

### Features

- [sol-macro] Add return value names to simple getters ([#648](https://github.com/alloy-rs/core/issues/648))

## [0.7.5](https://github.com/alloy-rs/core/releases/tag/v0.7.5) - 2024-06-04

### Bug Fixes

- [sol-macro] Allow deriving `Default` on contracts ([#645](https://github.com/alloy-rs/core/issues/645))
- [sol-macro] Overridden event signatures ([#642](https://github.com/alloy-rs/core/issues/642))

### Features

- [sol-macro] Allow overridden custom errors ([#644](https://github.com/alloy-rs/core/issues/644))

### Miscellaneous Tasks

- Temporarily disable tests that OOM Miri ([#637](https://github.com/alloy-rs/core/issues/637))

## [0.7.3](https://github.com/alloy-rs/core/releases/tag/v0.7.3) - 2024-05-14

### Miscellaneous Tasks

- Fix tests ([#624](https://github.com/alloy-rs/core/issues/624))
- Unused cfgs

## [0.7.2](https://github.com/alloy-rs/core/releases/tag/v0.7.2) - 2024-05-02

### Documentation

- Unhide and mention `sol!` wrappers ([#615](https://github.com/alloy-rs/core/issues/615))

### Miscellaneous Tasks

- [general] Add basic CI workflow for Windows ([#613](https://github.com/alloy-rs/core/issues/613))

## [0.7.1](https://github.com/alloy-rs/core/releases/tag/v0.7.1) - 2024-04-23

### Miscellaneous Tasks

- Update tests and clippy

## [0.7.0](https://github.com/alloy-rs/core/releases/tag/v0.7.0) - 2024-03-30

### Bug Fixes

- [json-abi] Correct to_sol for arrays of contracts ([#586](https://github.com/alloy-rs/core/issues/586))
- [sol-macro] Don't double attributes in JSON input ([#583](https://github.com/alloy-rs/core/issues/583))
- [sol-macro] Enumerate before filtering when expanding events ([#561](https://github.com/alloy-rs/core/issues/561))

### Features

- Rlp encoding for logs with generic event data ([#553](https://github.com/alloy-rs/core/issues/553))

### Performance

- [sol-macro] Decode bytecode hex strings ourselves ([#562](https://github.com/alloy-rs/core/issues/562))

### Styling

- Make `Bytes` map to `Bytes` in `SolType` ([#545](https://github.com/alloy-rs/core/issues/545))

## [0.6.4](https://github.com/alloy-rs/core/releases/tag/v0.6.4) - 2024-02-29

### Bug Fixes

- [dyn-abi] Correctly parse empty lists of bytes ([#548](https://github.com/alloy-rs/core/issues/548))

### Features

- Add `TxKind` ([#542](https://github.com/alloy-rs/core/issues/542))

### Miscellaneous Tasks

- Remove unused imports ([#534](https://github.com/alloy-rs/core/issues/534))

### Testing

- Add another ABI encode test ([#547](https://github.com/alloy-rs/core/issues/547))
- Bless tests ([#530](https://github.com/alloy-rs/core/issues/530))

## [0.6.3](https://github.com/alloy-rs/core/releases/tag/v0.6.3) - 2024-02-15

### Bug Fixes

- [json-abi] Accept nameless `Param`s ([#526](https://github.com/alloy-rs/core/issues/526))
- Properly test ABI packed encoding ([#517](https://github.com/alloy-rs/core/issues/517))
- Don't validate when decoding revert reason ([#511](https://github.com/alloy-rs/core/issues/511))

### Dependencies

- Recursion mitigations ([#495](https://github.com/alloy-rs/core/issues/495))

### Features

- [sol-types] Constify type name formatting ([#520](https://github.com/alloy-rs/core/issues/520))
- [sol-macro] Expand state variable getters in contracts ([#514](https://github.com/alloy-rs/core/issues/514))
- Make some allocations fallible in ABI decoding ([#513](https://github.com/alloy-rs/core/issues/513))

### Miscellaneous Tasks

- [sol-macro] Tweak inline attributes in generated code ([#505](https://github.com/alloy-rs/core/issues/505))

### Performance

- [sol-macro] Use `binary_search` in `SolInterface::valid_selector` ([#506](https://github.com/alloy-rs/core/issues/506))

### Testing

- Bless tests ([#524](https://github.com/alloy-rs/core/issues/524))
- Remove unused test ([#504](https://github.com/alloy-rs/core/issues/504))

## [0.6.1](https://github.com/alloy-rs/core/releases/tag/v0.6.1) - 2024-01-25

### Bug Fixes

- Deserialize missing state mutability as non payable ([#488](https://github.com/alloy-rs/core/issues/488))

### Documentation

- Remove stray list element ([#500](https://github.com/alloy-rs/core/issues/500))
- Fixes ([#498](https://github.com/alloy-rs/core/issues/498))

### Features

- Add constructorCall to `sol!` ([#493](https://github.com/alloy-rs/core/issues/493))
- [primitives] Add `Address::from_private_key` ([#483](https://github.com/alloy-rs/core/issues/483))

### Miscellaneous Tasks

- Include path in error ([#486](https://github.com/alloy-rs/core/issues/486))

## [0.6.0](https://github.com/alloy-rs/core/releases/tag/v0.6.0) - 2024-01-10

### Features

- [primitives] Add Keccak256 hasher struct ([#469](https://github.com/alloy-rs/core/issues/469))

### Miscellaneous Tasks

- Bless tests ([#478](https://github.com/alloy-rs/core/issues/478))
- Clippy uninlined_format_args, use_self ([#475](https://github.com/alloy-rs/core/issues/475))
- Move define_udt! decl macro to sol! proc macro ([#471](https://github.com/alloy-rs/core/issues/471))

### Refactor

- Log implementation ([#465](https://github.com/alloy-rs/core/issues/465))

## [0.5.4](https://github.com/alloy-rs/core/releases/tag/v0.5.4) - 2023-12-27

### Miscellaneous Tasks

- Clippy ([#463](https://github.com/alloy-rs/core/issues/463))
- [sol-types] Make PanicKind non_exhaustive ([#458](https://github.com/alloy-rs/core/issues/458))

## [0.5.3](https://github.com/alloy-rs/core/releases/tag/v0.5.3) - 2023-12-16

### Bug Fixes

- [sol-types] Un-break decode revert ([#457](https://github.com/alloy-rs/core/issues/457))

### Features

- Add `RevertReason` enum ([#450](https://github.com/alloy-rs/core/issues/450))
- [primitives] Update Bytes formatting, add UpperHex ([#446](https://github.com/alloy-rs/core/issues/446))

### Miscellaneous Tasks

- Bless tests ([#456](https://github.com/alloy-rs/core/issues/456))

## [0.5.2](https://github.com/alloy-rs/core/releases/tag/v0.5.2) - 2023-12-01

### Testing

- Add some regression tests ([#443](https://github.com/alloy-rs/core/issues/443))

## [0.5.0](https://github.com/alloy-rs/core/releases/tag/v0.5.0) - 2023-11-23

### Bug Fixes

- [sol-types] Many ABI coder fixes ([#434](https://github.com/alloy-rs/core/issues/434))
- [sol-types] ContractError decoding ([#430](https://github.com/alloy-rs/core/issues/430))
- [sol-macro] Handle outer attrs in abigen input ([#429](https://github.com/alloy-rs/core/issues/429))
- [sol-macro] Correctly print Custom types in parameters ([#425](https://github.com/alloy-rs/core/issues/425))
- [sol-types] Remove `SolType::ENCODED_SIZE` default ([#418](https://github.com/alloy-rs/core/issues/418))
- [syn-solidity] Raw keyword identifiers ([#415](https://github.com/alloy-rs/core/issues/415))
- Rust keyword conflict ([#405](https://github.com/alloy-rs/core/issues/405))
- [dyn-abi] Handle empty hex strings ([#400](https://github.com/alloy-rs/core/issues/400))
- [syn-solidity] Allow some duplicate attributes ([#399](https://github.com/alloy-rs/core/issues/399))
- Avoid symlinks ([#396](https://github.com/alloy-rs/core/issues/396))
- [sol-types] `SolInterface::MIN_DATA_LENGTH` overflow ([#383](https://github.com/alloy-rs/core/issues/383))
- [docs] Switch incorrect function docs ([#374](https://github.com/alloy-rs/core/issues/374))
- [sol-macro] Bug fixes ([#372](https://github.com/alloy-rs/core/issues/372))
- [sol-macro] Correct `SolCall::abi_decode_returns` ([#367](https://github.com/alloy-rs/core/issues/367))

### Features

- [sol-types] Add empty `bytes` and `string` specialization ([#435](https://github.com/alloy-rs/core/issues/435))
- [sol-macro] `SolEventInterface`: `SolInterface` for contract events enum ([#426](https://github.com/alloy-rs/core/issues/426))
- [sol-macro] Add `json-abi` item generation ([#422](https://github.com/alloy-rs/core/issues/422))
- [sol-types] Add some more methods to `abi::Decoder` ([#404](https://github.com/alloy-rs/core/issues/404))
- [dyn-abi] `DynSolType::coerce_str` ([#380](https://github.com/alloy-rs/core/issues/380))

### Miscellaneous Tasks

- Restructure tests ([#421](https://github.com/alloy-rs/core/issues/421))
- Rename `TokenType` GAT and trait to `Token` ([#417](https://github.com/alloy-rs/core/issues/417))
- Use winnow `separated` instead of `separated0` ([#403](https://github.com/alloy-rs/core/issues/403))
- Clean up ABI, EIP-712, docs ([#373](https://github.com/alloy-rs/core/issues/373))
- [sol-types] Remove impls for isize/usize ([#362](https://github.com/alloy-rs/core/issues/362))

### Styling

- Update rustfmt config ([#406](https://github.com/alloy-rs/core/issues/406))

## [0.4.1](https://github.com/alloy-rs/core/releases/tag/v0.4.1) - 2023-10-09

### Bug Fixes

- [sol-macro] Correct `TypeArray::is_abi_dynamic` ([#353](https://github.com/alloy-rs/core/issues/353))
- [sol-macro] Dedup json abi items ([#346](https://github.com/alloy-rs/core/issues/346))

### Features

- [sol-macro] Improve error messages ([#345](https://github.com/alloy-rs/core/issues/345))
- [sol-types] Introduce `SolValue`, make `Encodable` an impl detail ([#333](https://github.com/alloy-rs/core/issues/333))
- Add parsing support for JSON items ([#329](https://github.com/alloy-rs/core/issues/329))
- Add logs, add log dynamic decoding ([#271](https://github.com/alloy-rs/core/issues/271))

### Miscellaneous Tasks

- [sol-types] Rewrite encodable impl generics ([#332](https://github.com/alloy-rs/core/issues/332))
- Add count to all_the_tuples! macro ([#331](https://github.com/alloy-rs/core/issues/331))

### Other

- Run miri in ci ([#327](https://github.com/alloy-rs/core/issues/327))

### Testing

- Add regression test for [#351](https://github.com/alloy-rs/core/issues/351) ([#355](https://github.com/alloy-rs/core/issues/355))

## [0.4.0](https://github.com/alloy-rs/core/releases/tag/v0.4.0) - 2023-09-29

### Bug Fixes

- [sol-macro] Implement EventTopic for generated enums ([#320](https://github.com/alloy-rs/core/issues/320))
- Add super import on generated modules ([#307](https://github.com/alloy-rs/core/issues/307))
- Struct `eip712_data_word` ([#258](https://github.com/alloy-rs/core/issues/258))
- [syn-solidity] Imports ([#252](https://github.com/alloy-rs/core/issues/252))

### Documentation

- Data types typo ([#248](https://github.com/alloy-rs/core/issues/248))

### Features

- [sol-macro] Add support for overloaded events ([#318](https://github.com/alloy-rs/core/issues/318))
- [sol-macro] Improve type expansion ([#302](https://github.com/alloy-rs/core/issues/302))
- Improve `SolError`, `SolInterface` structs and implementations ([#285](https://github.com/alloy-rs/core/issues/285))
- Use `FixedBytes` for `sol_data::FixedBytes` ([#276](https://github.com/alloy-rs/core/issues/276))
- [sol-macro] Expand getter functions' return types ([#262](https://github.com/alloy-rs/core/issues/262))
- Add attributes to enum variants ([#264](https://github.com/alloy-rs/core/issues/264))
- [sol-macro] Expand fields with attrs ([#263](https://github.com/alloy-rs/core/issues/263))
- [dyn-abi] Implement more ext traits for json-abi ([#243](https://github.com/alloy-rs/core/issues/243))
- [sol-macro] Add opt-in attributes for extra methods and derives ([#250](https://github.com/alloy-rs/core/issues/250))

### Miscellaneous Tasks

- Prefix ABI encode and decode functions with `abi_` ([#311](https://github.com/alloy-rs/core/issues/311))
- Simpler ENCODED_SIZE for SolType tuples ([#312](https://github.com/alloy-rs/core/issues/312))
- Sync crate level attributes ([#303](https://github.com/alloy-rs/core/issues/303))
- Use `hex!` macro from `primitives` re-export ([#299](https://github.com/alloy-rs/core/issues/299))
- Do not implement SolType for SolStruct generically ([#275](https://github.com/alloy-rs/core/issues/275))
- Rename coding functions ([#274](https://github.com/alloy-rs/core/issues/274))

### Performance

- Use `slice::Iter` where possible ([#256](https://github.com/alloy-rs/core/issues/256))

### Refactor

- Simplify `Eip712Domain::encode_data` ([#277](https://github.com/alloy-rs/core/issues/277))

### Styling

- Format code snippets in docs ([#313](https://github.com/alloy-rs/core/issues/313))
- Move `decode_revert_reason` to alloy and add tests ([#308](https://github.com/alloy-rs/core/issues/308))
- Some clippy lints ([#251](https://github.com/alloy-rs/core/issues/251))

## [0.3.2](https://github.com/alloy-rs/core/releases/tag/v0.3.2) - 2023-08-23

### Bug Fixes

- [sol-macro] Snake_case'd function names ([#226](https://github.com/alloy-rs/core/issues/226))
- [sol-macro] Encode UDVTs as their underlying type in EIP-712 ([#220](https://github.com/alloy-rs/core/issues/220))

### Features

- [syn-solidity] Add statements and expressions ([#199](https://github.com/alloy-rs/core/issues/199))
- Function type ([#224](https://github.com/alloy-rs/core/issues/224))
- [dyn-abi] Allow `T: Into<Cow<str>>` in `eip712_domain!` ([#222](https://github.com/alloy-rs/core/issues/222))
- [sol-macro] Expand getter functions for public state variables ([#218](https://github.com/alloy-rs/core/issues/218))

### Performance

- Optimize some stuff ([#231](https://github.com/alloy-rs/core/issues/231))

## [0.3.1](https://github.com/alloy-rs/core/releases/tag/v0.3.1) - 2023-07-30

### Documentation

- Add ambiguity details to Encodable rustdoc ([#211](https://github.com/alloy-rs/core/issues/211))

## [0.3.0](https://github.com/alloy-rs/core/releases/tag/v0.3.0) - 2023-07-26

### Bug Fixes

- Correct encodeType expansion for nested structs ([#203](https://github.com/alloy-rs/core/issues/203))
- Remove unused method body on solstruct ([#200](https://github.com/alloy-rs/core/issues/200))
- [sol-types] Empty data decode ([#159](https://github.com/alloy-rs/core/issues/159))

### Features

- [sol-macro] `#[sol]` attributes and JSON ABI support ([#173](https://github.com/alloy-rs/core/issues/173))
- Solidity type parser ([#181](https://github.com/alloy-rs/core/issues/181))
- [dyn-abi] Add arbitrary impls and proptests ([#175](https://github.com/alloy-rs/core/issues/175))
- [dyn-abi] Cfg CustomStruct for eip712, rm CustomValue ([#178](https://github.com/alloy-rs/core/issues/178))
- [dyn-abi] Clean up and improve performance ([#174](https://github.com/alloy-rs/core/issues/174))
- [json-abi] Add more impls ([#164](https://github.com/alloy-rs/core/issues/164))
- `SolEnum` and `SolInterface` ([#153](https://github.com/alloy-rs/core/issues/153))
- [primitives] Fixed bytes macros ([#156](https://github.com/alloy-rs/core/issues/156))

### Miscellaneous Tasks

- Warn on all rustdoc lints ([#154](https://github.com/alloy-rs/core/issues/154))
- Clean ups ([#150](https://github.com/alloy-rs/core/issues/150))
- Add smaller image for favicon ([#142](https://github.com/alloy-rs/core/issues/142))
- Move macro doctests to separate folder ([#140](https://github.com/alloy-rs/core/issues/140))

### Other

- Significant dyn-abi fixes :) ([#168](https://github.com/alloy-rs/core/issues/168))

### Refactor

- Rename domain macro and add docs ([#147](https://github.com/alloy-rs/core/issues/147))
- Rename Sol*::Tuple to Parameters/Arguments  ([#145](https://github.com/alloy-rs/core/issues/145))
- Do not generate SolCall for return values ([#134](https://github.com/alloy-rs/core/issues/134))

### Testing

- Run UI tests only on nightly ([#194](https://github.com/alloy-rs/core/issues/194))

## [0.2.0](https://github.com/alloy-rs/core/releases/tag/v0.2.0) - 2023-06-23

### Bug Fixes

- Remove to_rust from most traits ([#133](https://github.com/alloy-rs/core/issues/133))
- (u)int tokenization ([#123](https://github.com/alloy-rs/core/issues/123))
- Make detokenize infallible ([#86](https://github.com/alloy-rs/core/issues/86))
- Type check int for dirty high bytes ([#47](https://github.com/alloy-rs/core/issues/47))

### Features

- Add `Encodable` trait ([#121](https://github.com/alloy-rs/core/issues/121))
- Finish high-level Solidity parser ([#119](https://github.com/alloy-rs/core/issues/119))
- Improve SolType tuples ([#115](https://github.com/alloy-rs/core/issues/115))
- Make `TokenType::is_dynamic` a constant ([#114](https://github.com/alloy-rs/core/issues/114))
- More FixedBytes impls ([#111](https://github.com/alloy-rs/core/issues/111))
- Compute encoded size statically where possible ([#105](https://github.com/alloy-rs/core/issues/105))
- Solidity events support ([#83](https://github.com/alloy-rs/core/issues/83))
- `sol!` contracts ([#77](https://github.com/alloy-rs/core/issues/77))
- Syn-solidity visitors ([#68](https://github.com/alloy-rs/core/issues/68))
- Abi benchmarks ([#57](https://github.com/alloy-rs/core/issues/57))
- Move Solidity syn AST to `syn-solidity` ([#63](https://github.com/alloy-rs/core/issues/63))
- Support function overloading in `sol!` ([#53](https://github.com/alloy-rs/core/issues/53))
- Primitive utils and improvements ([#52](https://github.com/alloy-rs/core/issues/52))
- Add PanicKind enum ([#54](https://github.com/alloy-rs/core/issues/54))

### Miscellaneous Tasks

- Add logo to all crates, add @gakonst to CODEOWNERS ([#138](https://github.com/alloy-rs/core/issues/138))
- Typos ([#132](https://github.com/alloy-rs/core/issues/132))
- Rename to Alloy ([#69](https://github.com/alloy-rs/core/issues/69))
- Enable `feature(doc_cfg, doc_auto_cfg)` ([#67](https://github.com/alloy-rs/core/issues/67))
- Rename crates ([#45](https://github.com/alloy-rs/core/issues/45))

### Other

- Revert "test: bless tests after updating to syn 2.0.19 ([#79](https://github.com/alloy-rs/core/issues/79))" ([#80](https://github.com/alloy-rs/core/issues/80))
- Fix rustdoc job, docs ([#46](https://github.com/alloy-rs/core/issues/46))

### Performance

- Improve rlp, update Address methods ([#118](https://github.com/alloy-rs/core/issues/118))

### Refactor

- Lifetimes for token types ([#120](https://github.com/alloy-rs/core/issues/120))
- Sol-macro expansion ([#113](https://github.com/alloy-rs/core/issues/113))
- Change is_dynamic to a const DYNAMIC ([#99](https://github.com/alloy-rs/core/issues/99))
- Implement `SolType` for `{Ui,I}nt<N>` and `FixedBytes<N>` with const-generics ([#92](https://github.com/alloy-rs/core/issues/92))

### Testing

- Bless tests after updating to syn 2.0.19 ([#79](https://github.com/alloy-rs/core/issues/79))

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
