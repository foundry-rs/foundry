# Changelog

## [1.1.0] - 2025-06-05

### Added
- Added `release.sh` for easy release automation and updated README.mds (#146) (filip-parity) - 2025-06-04
- Added cast & forge .sh and Dockerfile for release-test (#155) (filip-parity) - 2025-06-04
- Added fork documentation (#134) (filip-parity) - 2025-05-28
- Added cast serial tests on kittchensink node (#107) (ADPs) - 2025-05-28
- Added custom resolc settings (#123) (Sebastian Miasojed) - 2025-05-20
- Added forge tests (#118) (Sebastian Miasojed) - 2025-05-12
- Added test for compilation to the same output dir (#105) (Sebastian Miasojed) - 2025-05-09
- Added substrate-node to CI (#85) (Sebastian Miasojed) - 2025-04-15
- Added kittchensink node wrapper (#80) (Sebastian Miasojed) - 2025-04-11
- Added forge create tests (#72) (Sebastian Miasojed) - 2025-04-01
- Added revive build unit test (#68) (Sebastian Miasojed) - 2025-03-27
- Added GHA Scripts for generating and installing Foundry forge and cast builds (#65) (Ashish Peters) - 2025-03-27

### Changed
- Renamed foundryup to foundryup-polkadot (#138) (Sebastian Miasojed) - 2025-05-27
- Renamed revive to resolc (#95) (Sebastian Miasojed) - 2025-04-25
- Updated compilers version (#152, #147) (Sebastian Miasojed) - 2025-06-03, 2025-05-29
- Updated readme (#150) (Sebastian Miasojed) - 2025-06-02
- Updated docs (#145) (Pavlo Khrystenko) - 2025-05-29
- Updated foundry-compilers (#143) (Pavlo Khrystenko) - 2025-05-29
- Updated Compilers fork (#137) (Pavlo Khrystenko) - 2025-05-26
- Updated latest source for Cargo.lock > foundry-compilers-artifacts* (#127, #122) (filip-parity) - 2025-05-13, 2025-05-09
- Improved foundry revive config (#75) (Sebastian Miasojed) - 2025-04-08
- Improved error handling in the forge inspect cmd (#71) (Sebastian Miasojed) - 2025-03-28
- Improved CI and use dRPC service (#67) (Sebastian Miasojed) - 2025-03-25

### Fixed
- Fixed `forge compiler resolve` to accept `ResolcArgs` (#142) (Pavlo Khrystenko) - 2025-05-29
- Fixed the resolc config option propagation (#135) (Sebastian Miasojed) - 2025-05-22
- Fixed `forge bind` with `--resolc` (#132) (Pavlo Khrystenko) - 2025-05-21
- Fixed clippy CI issue (#94) (Pavlo Khrystenko) - 2025-04-24
- Fixed release yml (#110) (Pavlo Khrystenko) - 2025-04-30
- Fixed CI jobs timeout (#88) (Sebastian Miasojed) - 2025-04-16

### Infrastructure
- Use RVM to manage resolc versions (#96) (Sebastian Miasojed) - 2025-04-28
- Use rvm for `resolc` management (#87) (Pavlo Khrystenko) - 2025-04-25
- Enable CI for macOS (#74) (Sebastian Miasojed) - 2025-04-03
- Increase CI jobs timeout (#88) (Sebastian Miasojed) - 2025-04-16

### Documentation
- Init installation docs (#116) (Pavlo Khrystenko) - 2025-05-21
- Build manpages (#114) (Pavlo Khrystenko) - 2025-05-06

## Pre 1.0

### Important note for users

Multiple breaking changes will occur so Semver can be followed as soon as Foundry 1.0 is released. They will be listed here, along with the updates needed for your projects.

If you need a stable Foundry version, we recommend using the latest pinned nightly of May 2nd, locally and on your CI.

To use the latest pinned nightly locally, use the following command:

```
foundryup --version nightly-e15e33a07c0920189fc336391f538c3dad53da73
````

To use the latest pinned nightly on your CI, modify your Foundry installation step to use an specific version:

```
- name: Install Foundry
  uses: foundry-rs/foundry-toolchain@v1
  with:
    version: nightly-e15e33a07c0920189fc336391f538c3dad53da73
```

### Breaking changes

- [expectEmit](https://github.com/foundry-rs/foundry/pull/4920) will now only work for the next call.
- expectCall will now only work if the call(s) are made exactly after the cheatcode is invoked.
- [expectRevert will now work if the next call does revert](https://github.com/foundry-rs/foundry/pull/4945), instead of expecting a revert during the whole test.
  - This will very likely break your tests. Please make sure that all the calls you expect to revert are external, and if not, abstract them into a separate contract so that they can be called externally and the cheatcode can be used.
- `-m`, the deprecated alias for `--mt` or `--match-test`, has now been removed.
- [startPrank will now override the existing prank instead of erroring](https://github.com/foundry-rs/foundry/pull/4826).
- [precompiles will not be compatible with all cheatcodes](https://github.com/foundry-rs/foundry/pull/4905).
- The difficulty and prevrandao cheatcodes now [fail if not used with the correct EVM version](https://github.com/foundry-rs/foundry/pull/4904).
- The default EVM version will be Shanghai. If you're using an EVM chain which is not compatible with [EIP-3855](https://eips.ethereum.org/EIPS/eip-3855) you need to change your EVM version. See [Matt Solomon's thread](https://twitter.com/msolomon44/status/1656411871635972096) for more information.
- Non-existent JSON keys are now processed correctly, and `parseJson` returns non-decodable empty bytes if they do not exist. https://github.com/foundry-rs/foundry/pull/5511
