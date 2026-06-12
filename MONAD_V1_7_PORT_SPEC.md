# Monad Foundry v1.7 Port Work Spec

## Status

- Foundry branch: `v1.7.0-monad`
- Foundry base: upstream `v1.7.0`
- Foundry base commit: `f83bad912a9dba7bf0371def1e70bb1896048356`
- Monad reference branch: `monad`
- Monad reference commit: `1b981d375f95d23bfd5c1659eb351900c8963c4d`
- Upstream release notes: <https://github.com/foundry-rs/foundry/releases/tag/v1.7.0>
- Date written: 2026-05-04
- Release decision: complete this port by merging `v1.7.0-monad` into `monad`, then cut the
  first v1.7 release from an upstream `v1.7.1` base.

This document specifies the work required to rebuild Monad Foundry on top of upstream Foundry
`v1.7.0`.

The intended strategy is to start from upstream `v1.7.0` and port the Monad behavior forward,
using the existing `monad` branch as the reference implementation. The work should not be
implemented by merging upstream into the old `monad` branch and resolving conflicts in place.

## Branch Topology

The v1.7 Monad release line has three integration branches. Feature work should branch from these
branches and PR back into them. They are the final staging branches before pinning commits and
cutting a release.

| Repository | Local path | Integration branch | Current anchor | Role |
| --- | --- | --- | --- | --- |
| Foundry | `/Users/haythem/Projects/Toolings/foundry` | `v1.7.0-monad` | `f83bad912a9dba7bf0371def1e70bb1896048356` | Top-level Foundry port and release branch, based on upstream Foundry `v1.7.0`. |
| monad-revm | `/Users/haythem/Projects/Category/monad-revm` | `v1.7.0-monad` | `821b8edecaea510c33ad9303c753b4286802caa4` | Monad execution rules, context, journal, hardfork, and low-level precompile compatibility branch. |
| alloy-monad-evm | `/Users/haythem/Projects/Category/alloy-monad-evm` | `v1.7.0-monad` | `7b5f11984275f05e7b0c1e9af90e942a80bac559` | Alloy EVM adapter, Monad EVM factory, and Foundry-facing precompile compatibility branch. |

Dependency flow should move upward:

1. Land `revm 38` compatibility and Monad precompile-provider compatibility in
   `monad-revm:v1.7.0-monad`.
2. Land `alloy-evm 0.33.2` compatibility in `alloy-monad-evm:v1.7.0-monad`, depending on the
   compatible `monad-revm` branch or commit.
3. Land the Foundry integration in `foundry:v1.7.0-monad`, pinned to the compatible
   `monad-revm` and `alloy-monad-evm` branch commits.

Release readiness requires all three branches to be green together. The final Foundry release
should pin exact dependency commits or published versions from the two Monad EVM repos before any
immutable Monad Foundry release tag is created.

## Versioning and Release Target

The `v1.7.0-monad` branch is a port-staging branch, not the final release branch. Once the port is
green, merge it back into `monad`, update the release base to upstream Foundry `v1.7.1`, and cut the
first v1.7 Monad release from that line.

Immutable Monad Foundry tags use this shape:

```text
v<upstream-foundry-version>-monad-v<monad-foundry-version>
```

For the first v1.7 release, the expected tag is:

```text
v1.7.1-monad-v1.0.0
```

This keeps the upstream Foundry base explicit while giving the Monad fork an independent release
version. It is clearer than `v1.7.1-monad.1.0.0`, where the Monad release version looks like a set
of extra prerelease identifiers attached directly to the upstream version.

CLI version output should drop only the Git tag's leading `v`; for example a
`v1.7.1-monad-v1.0.0` release should print `1.7.1-monad-v1.0.0`.

`stable-monad` may remain as a moving installer alias for convenience, but immutable release tags
are the source of truth.

## Executive Summary

Upstream Foundry `v1.7.0` contains large architectural changes since the Monad fork point. The
most important change is the generic `Network` / `EvmFactory` architecture that allows a single
Foundry binary to support Ethereum, Optimism, and Tempo execution paths. This architecture is
directly relevant to Monad and should become the foundation of the new Monad Foundry branch.

The existing Monad branch adds:

- Monad EVM execution through `monad-revm` and `alloy-monad-evm`.
- Monad-specific gas behavior, runtime code size, initcode size, and hardfork selection.
- Monad staking and reserve-balance precompile support.
- Monad staking cheatcodes at a separate cheatcode address.
- Monad hardfork config and `vm.setEvmVersion` support for `MonadEight`, `MonadNine`, and
  `MonadNext`.
- `anvil --monad`, Monad fork detection, and Monad hardfork selection.
- Monad trace decoding and verify/compiler behavior.
- Monad installer, release, documentation, and CI changes.

Upstream `v1.7.0` adds and changes:

- Generic `FoundryEvmNetwork`, `FoundryEvmFactory`, `FoundryHardfork`, and `NetworkConfigs`.
- A new `foundry-evm-hardforks` crate.
- `network` and namespaced `hardfork` config keys.
- `revm 38`, `alloy-evm 0.33.2`, `op-revm 19`, and a significantly newer dependency graph.
- Extensive changes to `foundry-evm-core`, `foundry-evm`, cheatcodes, Anvil, Forge runner,
  script, cast, traces, config, and tests.
- Fuzzing, invariant, tracing, coverage, browser-wallet, MPP, Tempo, and release-system changes.

The port should adopt upstream v1.7.0 internals and add Monad as a first-class network family
instead of preserving the older v1.5-era Monad-only execution shape.

## Measured Local Context

These measurements were taken before creating this branch:

- Existing `monad` branch was based on an upstream post-1.5 commit, not exactly `v1.5.0`.
- Merge base with upstream v1.7.0: `fda3fec1f6216127d0969a1270f92759d576ae09`.
- Upstream delta from that merge base to `v1.7.0`: 1007 commits, 644 files,
  `+62898 / -34029`.
- Monad delta from that merge base to `monad`: 74 commits, 129 files,
  `+5140 / -4350`.
- Files touched by both upstream and Monad from the shared base: 111.
- Dry-run merge of `v1.7.0` into `monad`: 75 conflict paths.
- Highest conflict concentration:
  - `crates/evm/core`
  - `crates/anvil/src`
  - `crates/cheatcodes/src`
  - `crates/evm/evm`
  - `crates/forge/src`
  - `crates/forge/tests`
  - `Cargo.toml` / `Cargo.lock`
  - CI and release workflows

This confirms that the port is feasible, but it is a structured integration project rather than a
small merge.

## Goals

1. Build Monad Foundry on top of upstream `v1.7.0`.
2. Preserve the user-facing Monad behavior from the current `monad` branch.
3. Preserve upstream `v1.7.0` behavior for Ethereum, Optimism, Tempo, and generic Foundry usage.
4. Treat Monad as a first-class network family in the v1.7 architecture.
5. Minimize long-lived fork-specific divergence where upstream now provides a generic extension
   point.
6. Keep existing Monad tests and add new tests for any behavior moved into upstream v1.7
   abstractions.
7. Produce a releasable `v<upstream-foundry-version>-monad-v<monad-foundry-version>` line with
   clear installer and tag semantics.

## Non-Goals

- Do not redesign Monad protocol rules in this branch.
- Do not rewrite `monad-revm` or `alloy-monad-evm` beyond compatibility work required for
  upstream v1.7 dependencies.
- Do not remove upstream Tempo support.
- Do not collapse Monad behavior into Tempo behavior. Use Tempo as an architectural reference,
  but keep Monad as a distinct network.
- Do not blindly cherry-pick all old Monad commits. Port behavior intentionally onto v1.7.0
  APIs.
- Do not attempt to upstream the Monad fork into `foundry-rs/foundry` as part of this branch.

## Source of Truth

Use the current `monad` branch as the behavioral source of truth.

Important Monad commits to inspect and port by feature group:

- `a828eb063` - initial `monad-foundry` integration.
- `5329e3b14` - Monad integration with forge, verify, chisel, and cast.
- `737347351` - Monad contract size limit.
- `592df938f` - preserve config settings when applying Monad gas params.
- `45995903b` - staking precompile ABI decoding, traces, and linked-list traversal.
- `a0fc869a9` - `foundryup` Monad network support.
- `a1e71ce55` - Monad staking cheatcodes.
- `f685256b6` - `monad_hardfork` config option for MIP-3.
- `33f26ef88` - reject invalid `monad_hardfork` and propagate spec.
- `32955a59c` - Anvil Monad hardfork selection.
- `8be003895` - Monad hardfork names in `vm.setEvmVersion`.
- `13ef98394` - MIP-4 reserve-balance precompile support.
- `d32a8511e` - MonadNine expectation refresh.
- `1b981d375` - documentation of Monad hardfork default.

Treat these commits as evidence of required behavior, not as a recommended cherry-pick order.

## Target User-Facing Behavior

### Default Tools

- `forge test` should execute with Monad EVM behavior by default in Monad Foundry builds unless
  an explicit non-Monad network mode is selected.
- `forge script` should execute with Monad EVM behavior by default in Monad Foundry builds unless
  an explicit non-Monad network mode is selected.
- `cast run` and call simulation paths should use Monad EVM behavior where applicable.
- `chisel` should use Monad EVM behavior by default in Monad Foundry builds.
- `forge verify-contract` should preserve Monad-specific compiler and verification behavior from
  the existing fork.

### Network Selection

Support both the old and new selection models:

- Preferred v1.7 model: `network = "monad"` in `foundry.toml`.
- Preferred v1.7 CLI: `--network monad`.
- Backward-compatible CLI alias: `--monad`.
- Fork auto-detection: fork URLs whose chain ID resolves to `NamedChain::Monad` or
  `NamedChain::MonadTestnet` should activate Monad behavior.

### Hardfork Selection

The target model should align with upstream v1.7 namespaced hardforks:

- Preferred config: `hardfork = "monad:MonadNine"`.
- Backward-compatible config: `monad_hardfork = "MonadNine"`.
- Supported hardforks:
  - `MonadEight`
  - `MonadNine`
  - `MonadNext`
- Default: `MonadNine`, unless protocol requirements change before release.

Conflict rules:

- `network = "monad"` with `hardfork = "monad:MonadNine"` is valid.
- `network = "monad"` with an Ethereum-only hardfork should be rejected or normalized only if the
  old Monad branch intentionally allowed that path.
- `network = "tempo"` with `hardfork = "monad:MonadNine"` must be rejected.
- `monad_hardfork` and `hardfork = "monad:..."` must not conflict silently. If both are set and
  differ, config loading should fail with a clear error.

### `vm.setEvmVersion`

Preserve current Monad behavior:

- `vm.setEvmVersion("MonadEight")`, `vm.setEvmVersion("MonadNine")`, and
  `vm.setEvmVersion("MonadNext")` should update the active Monad spec for subsequent execution.
- Ethereum EVM version names may remain accepted for compatibility, but on Monad they must not
  accidentally switch the active Monad hardfork unless this is intentionally redesigned and tested.

### Anvil

Support:

- `anvil --monad`
- `anvil --network monad`
- `anvil --monad --hardfork MonadNine`
- `anvil --network monad --hardfork monad:MonadNine`
- Monad behavior when forking Monad RPC endpoints.
- Correct node info reporting so downstream tools can infer Monad when forking local Anvil.

### Precompiles and System Behavior

Port the Monad behavior from the existing branch:

- Monad staking precompile support.
- Monad reserve-balance precompile support for MIP-4.
- Monad system address handling.
- Monad-specific gas costs.
- Monad-specific no-refund behavior.
- Monad runtime code size limit: 128 KB.
- Monad initcode size limit: 256 KB.
- No EIP-4844 blob transaction behavior unless Monad protocol support is explicitly added later.

### Cheatcodes

Port Monad staking cheatcodes:

- Keep the separate Monad cheatcode address.
- Keep `testdata/utils/MonadVm.sol`.
- Keep the behavior tested by `testdata/default/cheats/MonadStaking.t.sol`.
- Adapt implementation to upstream v1.7 generic cheatcode infrastructure.

## Target Internal Architecture

### Dependency Alignment

Current upstream v1.7 dependency anchors:

- `alloy-evm = "0.33.2"`
- `revm = "38.0.0"`
- `op-revm = "19.0.0"`
- `foundry-wallets = "0.1.0"`
- `foundry-evm-hardforks` exists as a workspace crate.

Current Monad branch anchors:

- `alloy-evm = "0.26.3"`
- `alloy-monad-evm = "0.3.0"`
- `revm = "34.0.0"`
- `op-revm = "15.0.0"`
- `monad-revm = "0.3.0"`
- `foundry-wallets` is a local crate.

The first technical milestone is compatibility between Monad crates and upstream v1.7 dependencies.
This work should start in `monad-revm` and `alloy-monad-evm` before the Foundry branch consumes
them. Foundry v1.7 already expects the newer `revm` and `alloy-evm` stack, so trying to port the
Foundry integration first will mostly surface dependency and trait-shape errors from the lower
layers.

Expected actions:

1. On `monad-revm:v1.7.0-monad`, update to the `revm 38` API and keep Monad context, journal,
   hardfork, staking, and reserve-balance behavior compiling against that API.
2. On `alloy-monad-evm:v1.7.0-monad`, update to the `alloy-evm 0.33.2` API and depend on the
   compatible `monad-revm` branch or commit.
3. Make `alloy-monad-evm::MonadEvmFactory` expose `type Precompiles = PrecompilesMap`.
4. Move or add Monad-specific `PrecompileProvider<MonadContext<DB>> for PrecompilesMap`
   compatibility in a Monad-owned crate.
5. Add `monad-revm` and `alloy-monad-evm` as workspace dependencies in upstream v1.7 style only
   after the two Monad EVM repos compile on their v1.7 branches.
6. Avoid reintroducing the local `crates/wallets` crate unless absolutely required.
7. Keep `Cargo.lock` coherent and avoid duplicated `revm`, `alloy`, and `foundry-core` versions.

Acceptance:

- `cargo metadata` succeeds.
- `cargo tree -d` does not show duplicate major/minor `revm` or `alloy-evm` stacks caused by
  Monad crates.
- `cargo check -p foundry-evm-hardforks -p foundry-evm-networks` succeeds before deeper EVM work.

### Hardfork Crate

File area:

- `crates/evm/hardforks/src/lib.rs`
- `crates/evm/hardforks/Cargo.toml`

Add Monad to upstream's `FoundryHardfork` model.

Target shape:

- Add `FoundryHardfork::Monad(MonadSpecId)` or a small `MonadHardfork` wrapper if direct
  dependency exposure is undesirable.
- Add namespace support:
  - `monad:MonadEight`
  - `monad:MonadNine`
  - `monad:MonadNext`
  - aliases may include `m:MonadNine` if useful.
- Add serialization as `monad:<name>`.
- Add deserialization from `monad:<name>`.
- Add conversion to Ethereum `SpecId` only for shared generic code paths that require it.
- Add conversion to `MonadSpecId` for Monad execution paths.
- Add tests for parsing, serialization, invalid names, and conflict messages.

Open design point:

- `FoundryHardfork::name()` currently returns an unqualified hardfork name. For Monad, this should
  return `MonadNine`; `String::from(FoundryHardfork::Monad(...))` should return
  `monad:MonadNine`.

### Network Configs

File area:

- `crates/evm/networks/src/lib.rs`
- `crates/cli/src/opts/evm.rs`
- `crates/config/src/lib.rs`

Add Monad to upstream's `NetworkVariant`:

- `NetworkVariant::Monad`
- `NetworkVariant::name() -> "monad"`
- `NetworkConfigs::with_monad()`
- `NetworkConfigs::is_monad()`
- `NetworkConfigs::active_network_name() -> Some("monad")`
- `NetworkConfigs::with_chain_id()` detects `NamedChain::Monad` and `NamedChain::MonadTestnet`.
- `NetworkConfigs::normalize_for_hardfork()` infers Monad from `FoundryHardfork::Monad`.
- CLI `--network monad` should work.
- CLI `--monad` should be retained as a hidden or visible compatibility flag, depending on desired
  UX.

Acceptance:

- Config tests prove `network = "monad"` parses.
- Config tests prove `hardfork = "monad:MonadNine"` implies Monad network when `network` is unset.
- Config tests prove conflicting hardfork/network settings fail.
- CLI tests prove `--network monad` and `--monad` both select Monad.

### Core EVM Integration

File area:

- `crates/evm/core/src/evm/mod.rs`
- `crates/evm/core/src/evm/monad.rs` or equivalent
- `crates/evm/core/src/env.rs`
- `crates/evm/core/src/backend/mod.rs`
- `crates/evm/core/src/backend/cow.rs`
- `crates/evm/core/src/fork/init.rs`
- `crates/evm/core/src/precompiles.rs`
- `crates/evm/core/src/opts.rs`

Use upstream v1.7's generic architecture as the target pattern.

Expected target:

- Add `MonadEvmNetwork` implementing `FoundryEvmNetwork`.
- Add or adapt a `MonadEvmFactory` implementing `FoundryEvmFactory`.
- Use `alloy-monad-evm` and `monad-revm` as the execution backend.
- Keep Monad-specific `Spec` as `MonadSpecId`.
- Keep Monad-specific context/journal types from `monad-revm`.
- Ensure Foundry's nested EVM support works for Monad cheatcodes, pranks, broadcasts, mocks,
  traces, and CREATE2 factory behavior.

Implementation guidance:

- Use upstream `crates/evm/core/src/evm/tempo.rs` as the closest architectural reference.
- Do not restore the old branch's global Monad-only `InspectorExt` if the upstream generic
  `FoundryInspectorExt<Context>` can express the behavior.
- Do not restore the old branch's root `crates/evm/core/src/evm.rs` shape if a new
  `evm/monad.rs` module is cleaner in v1.7.
- Preserve Monad-specific CREATE2 behavior and test it.
- Preserve gas-parameter application after config and chain-specific environment changes.

Acceptance:

- `cargo check -p foundry-evm-core --all-targets` succeeds.
- Existing Ethereum and Optimism EVM core tests continue to pass.
- New Monad core tests prove the selected `MonadSpecId` reaches execution.
- New Monad core tests prove code size and gas behavior differ from Ethereum where required.

### Precompiles

File area:

- `crates/evm/core/src/precompiles.rs`
- `crates/evm/traces/src/decoder/mod.rs`
- `crates/common/src/constants.rs`
- Monad precompile dependency crates

Port:

- Staking precompile integration.
- Reserve-balance precompile integration.
- ABI decoding for both precompiles.
- Precompile labels for traces.
- Correct behavior across forked and non-forked execution.

Acceptance:

- `testdata/default/cheats/MonadStaking.t.sol` passes.
- MIP-4 reserve-balance regression tests pass.
- Trace output decodes staking and reserve-balance calls with meaningful labels/selectors.

### Upstream-Compatible Precompile Shape

This is a gating design decision for the v1.7 port.

Upstream Foundry v1.7 currently requires every `FoundryEvmFactory` to satisfy:

```rust
EvmFactory<Precompiles = PrecompilesMap>
```

This appears in `crates/evm/core/src/evm/mod.rs` and is already used by Ethereum, Optimism, and
Tempo. If the goal is to make a future upstream PR as small and acceptable as possible, the Monad
port should avoid asking upstream to relax this bound.

Current local Monad crates do not yet satisfy this shape:

- `alloy-monad-evm::MonadEvmFactory` currently has `type Precompiles = MonadPrecompilesMap`.
- `MonadPrecompilesMap` wraps an inner `PrecompilesMap` and implements
  `PrecompileProvider<MonadContext<DB>>`.
- The wrapper exists for a real reason: Monad precompiles need Monad-specific dispatch before
  falling back to regular Ethereum-style precompiles.

The important distinction:

- Staking can mostly fit the `PrecompileInput` / `EvmInternals` model because
  `alloy-monad-evm` already has an adapter that reads/writes staking storage through
  `EvmInternals.sload`, `sstore`, `transfer`, and `log`.
- Reserve balance cannot be represented as a plain stateful `PrecompilesMap` closure without
  losing behavior, because it reads Monad journal state:

```rust
context.journal().reserve_balance().has_violation()
```

`alloy_evm::PrecompileInput` exposes generic EVM state hooks, but it does not expose the typed
Monad reserve-balance tracker. Therefore the correct compatibility layer is not to force reserve
balance into a generic closure. The correct layer is to make plain `PrecompilesMap` dispatch
through Monad-aware `PrecompileProvider<MonadContext<DB>>` behavior when the context is Monad.

Target Monad crate shape:

```rust
impl EvmFactory for MonadEvmFactory {
    type Precompiles = PrecompilesMap;
}
```

and, in a Monad-owned crate where Rust coherence permits it:

```rust
impl<DB: Database> PrecompileProvider<MonadContext<DB>> for PrecompilesMap {
    type Output = InterpreterResult;

    fn set_spec(&mut self, spec: MonadSpecId) -> bool {
        // Rebuild the map for MonadEight / MonadNine / MonadNext.
        // This must update repriced Ethereum precompiles and Monad-only addresses.
    }

    fn run(
        &mut self,
        context: &mut MonadContext<DB>,
        inputs: &CallInputs,
    ) -> Result<Option<Self::Output>, String> {
        if let Some(result) = staking::run_staking_precompile(context, inputs)? {
            return Ok(Some(result));
        }

        if let Some(result) = reserve_balance::run_reserve_balance_precompile(context, inputs)? {
            return Ok(Some(result));
        }

        // Fall back to normal PrecompilesMap execution for standard/repriced precompiles.
    }

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        // Include 0x1000, conditionally 0x1001 for MonadNine+, and standard/repriced addresses.
    }

    fn contains(&self, address: &Address) -> bool {
        // Include Monad-only addresses and normal PrecompilesMap addresses.
    }
}
```

Implementation options:

1. Prefer implementing the Monad-specific `PrecompileProvider<MonadContext<DB>> for PrecompilesMap`
   in `monad-revm` behind an optional `alloy-evm` feature, because `monad-revm` owns
   `MonadContext`.
2. If that is not practical due to dependency layering, implement it in `alloy-monad-evm`, but
   confirm Rust coherence allows it. The impl uses a foreign trait and foreign self type, so it
   must involve a Monad-owned local type in the trait parameters in a way the compiler accepts.
3. Avoid changing Foundry's `FoundryEvmFactory` bound unless both options above fail.

Required changes in `alloy-monad-evm`:

- Update dependencies to upstream v1.7 stack:
  - `alloy-evm = "0.33.2"`
  - `revm = "38.0.0"`
  - compatible `monad-revm`
- Change `MonadEvmFactory::Precompiles` from `MonadPrecompilesMap` to `PrecompilesMap`.
- Replace `MonadPrecompilesMap::new_with_spec(spec)` with a helper that returns a plain
  `PrecompilesMap` populated with:
  - Monad-repriced standard precompiles.
  - P256VERIFY when required.
  - Staking address `0x1000`.
  - Reserve-balance address `0x1001` for `MonadNine+`.
- Keep `extend_monad_precompiles(&mut PrecompilesMap)` or replace it with a clearer
  `monad_precompiles_map(spec: MonadSpecId) -> PrecompilesMap`.
- Ensure fallback dynamic precompile execution still uses `PrecompileInput` so staking read/write
  behavior through `EvmInternals` remains available if needed.

Required changes in `monad-revm`:

- Update to `revm 38`.
- Preserve `MonadContext`, `MonadJournal`, and `MonadContextTr`.
- Keep `staking::run_staking_precompile`.
- Keep `reserve_balance::run_reserve_balance_precompile`.
- Add tests proving `PrecompilesMap` with a `MonadContext`:
  - contains staking address for all Monad specs.
  - contains reserve-balance address only on `MonadNine+`.
  - dispatches staking before standard precompile fallback.
  - dispatches reserve balance and reads the Monad journal tracker.
  - rebuilds on `set_spec`.

Acceptance for this shape before touching Foundry:

- `alloy-monad-evm::MonadEvmFactory` compiles with `type Precompiles = PrecompilesMap`.
- Staking precompile tests pass.
- Reserve-balance precompile tests pass.
- `PrecompilesMap` warm addresses include Monad-only precompiles correctly.
- No Foundry-side change is required to the `FoundryEvmFactory` precompile bound.

This route is preferred for future upstreaming because the eventual Foundry PR can focus on adding
Monad as a network, hardfork family, factory, and test suite, without also asking upstream to
generalize the precompile-provider type.

### Cheatcodes

File area:

- `crates/cheatcodes/src/monad.rs`
- `crates/cheatcodes/src/lib.rs`
- `crates/cheatcodes/src/inspector.rs`
- `crates/cheatcodes/src/evm.rs`
- `crates/cheatcodes/assets/cheatcodes.json`
- `testdata/utils/MonadVm.sol`

Port the Monad staking cheatcode module and adapt it to v1.7 generic cheatcodes.

Required behavior:

- Dispatch calls sent to the Monad cheatcode address.
- Use the Monad context/journal/storage adapter required by `monad-revm`.
- Support:
  - `setEpoch`
  - `setProposer`
  - `setAccumulator`
  - `blockReward`
  - `epochSnapshot`
  - `epochChange`
  - `epochBoundary`
- Preserve error messages for unknown selectors where practical.
- Preserve safety and isolation semantics from standard cheatcodes.

Acceptance:

- Monad staking cheatcode Solidity tests pass.
- Existing upstream cheatcode tests pass.
- `vm.setEvmVersion` tests cover Monad names and Ethereum compatibility behavior.

### Forge, Script, Cast, Chisel, and Verify

File areas:

- `crates/forge/src/cmd/test/mod.rs`
- `crates/forge/src/multi_runner.rs`
- `crates/forge/src/runner.rs`
- `crates/script/src/*`
- `crates/cast/src/cmd/call.rs`
- `crates/cast/src/cmd/run.rs`
- `crates/cast/src/lib.rs`
- `crates/chisel/src/executor.rs`
- `crates/verify/src/bytecode.rs`
- `crates/verify/src/utils.rs`

Port behavior:

- Forge test runner creates Monad EVMs when Monad network is active.
- Forge script uses Monad execution and transaction assumptions when Monad is active.
- Cast call/run paths use Monad execution when Monad network is active.
- Chisel uses Monad execution by default in Monad builds.
- Verify uses Monad-specific compiler settings and bytecode comparison behavior.

Important upstream interaction:

- v1.7 script and broadcast code is generic over `Network`. Monad should fit this model.
- Browser wallet and Tempo additions should not be broken by Monad conditionals.
- Avoid adding Monad-only shortcuts into generic paths when a `FoundryEvmNetwork` type parameter
  can carry the behavior.

Acceptance:

- Forge CLI tests for build, test, script, and config pass.
- Cast CLI tests for call/run pass.
- Chisel integration tests pass or have documented non-Monad failures unrelated to this work.
- Verify tests pass and include Monad-specific cases.

### Anvil

File areas:

- `crates/anvil/src/cmd.rs`
- `crates/anvil/src/config.rs`
- `crates/anvil/src/eth/api.rs`
- `crates/anvil/src/eth/backend/*`
- `crates/anvil/src/evm.rs`
- `crates/anvil/src/eth/pool/*`
- `crates/anvil/tests/it/*`

Port behavior:

- `anvil --network monad` and `anvil --monad` select Monad.
- `--hardfork MonadNine` works in Monad mode.
- `--hardfork monad:MonadNine` works in Monad mode.
- Forking a Monad RPC selects Monad based on chain ID or node info.
- Block execution uses Monad gas, precompiles, hardfork, and transaction rules.
- Pool validation honors Monad rules.
- RPC transaction and receipt types remain compatible with clients.
- Anvil node info exposes enough network metadata for `forge` and `cast` inference.

Use upstream v1.7 Anvil generic patterns where possible. Do not copy old Monad-specific Anvil
execution wholesale if the upstream backend now provides a better insertion point.

Acceptance:

- `cargo test -p anvil --lib` succeeds.
- `cargo test -p anvil --test it` succeeds or known unrelated upstream flakes are documented.
- Monad Anvil MIP-3 and MIP-4 shell regressions pass.
- Fork-mode Monad tests pass against configured Monad RPC when credentials/network are available.

### Traces and Debugger

File areas:

- `crates/evm/traces/src/decoder/mod.rs`
- `crates/evm/traces/src/decoder/precompiles.rs`
- `crates/evm/evm/src/inspectors/stack.rs`
- `crates/evm/evm/src/inspectors/*`
- `crates/debugger/src/*`

Port behavior:

- Monad staking precompile calls decode in traces.
- Monad reserve-balance precompile calls decode in traces.
- Empty labels continue to be filtered.
- Monad trace behavior coexists with upstream v1.7 trace depth, console, and verbosity changes.

Acceptance:

- Trace CLI tests pass.
- Monad-specific trace fixtures or assertions prove precompile decoding.
- No regressions in upstream `expectRevert` formatting and console output behavior.

### Common Constants and Compiler Limits

File areas:

- `crates/common/src/constants.rs`
- `crates/common/src/compile.rs`
- `crates/config/src/lib.rs`

Port behavior:

- Monad system address is included where system senders are filtered or labeled.
- Runtime code size and initcode size limits use Monad limits when Monad is active.
- Ethereum limits remain unchanged when Monad is not active.

Acceptance:

- Unit tests cover Ethereum and Monad contract size limits.
- Existing compiler tests do not regress.

### Installer, Release, and CI

File areas:

- `foundryup/foundryup`
- `foundryup/install`
- `.github/workflows/*`
- `README.md`

Port behavior:

- `foundryup --network monad` installs Monad Foundry from the Category fork.
- Monad release tags use the intended `v<upstream-foundry-version>-monad-v<monad-foundry-version>`
  line, starting with `v1.7.1-monad-v1.0.0`.
- Upstream v1.7 immutable release model should be respected where practical.
- CI should run the upstream relevant test matrix plus Monad-specific regressions.

Decision:

- Keep `stable-monad` only as a fork-specific mutable convenience alias. Immutable release tags are
  authoritative and should follow `v<upstream-foundry-version>-monad-v<monad-foundry-version>`.

Acceptance:

- Release workflow can build all four binaries.
- Linux x86_64, Linux aarch64, macOS x86_64, and macOS aarch64 artifacts are produced.
- Installer resolves and downloads the new release artifacts.
- CI does not depend on upstream-only secrets.

## Port Order

The work should be landed in small compile-oriented slices. A recommended order:

### Phase 0: Baseline and Inventory

1. Confirm this branch is based on `v1.7.0`.
2. Save reference outputs from the current `monad` branch:
   - `forge --version`
   - `forge test` on Monad fixtures
   - Anvil Monad smoke tests
   - MIP-3 and MIP-4 shell regressions
   - selected trace outputs
3. Record exact `monad-revm` and `alloy-monad-evm` versions or commits planned for the v1.7 port.

Exit criteria:

- Baseline behavior is documented.
- External dependency strategy is decided.

### Phase 1: Dependency Compatibility

1. Add Monad dependencies to `Cargo.toml`.
2. Align `monad-revm` with `revm 38`.
3. Align `alloy-monad-evm` with `alloy-evm 0.33.2`.
4. Make `alloy-monad-evm::MonadEvmFactory` expose `type Precompiles = PrecompilesMap`.
5. Move or add Monad-specific `PrecompileProvider<MonadContext<DB>> for PrecompilesMap`
   compatibility in the Monad crates.
6. Update `Cargo.lock`.
7. Run metadata and minimal checks.

Exit criteria:

- `cargo metadata` succeeds.
- No duplicate incompatible `revm` or `alloy-evm` stacks.
- `MonadEvmFactory` satisfies Foundry v1.7's existing `PrecompilesMap` factory bound.
- Staking and reserve-balance tests pass in the Monad crates before Foundry integration starts.

### Phase 2: Hardfork and Network Config

1. Add Monad hardfork parsing and serialization.
2. Add `NetworkVariant::Monad`.
3. Add `network = "monad"` support.
4. Add `--network monad`.
5. Add compatibility support for `--monad`.
6. Add compatibility support for `monad_hardfork`.
7. Add config conflict validation.

Exit criteria:

- Config and network crates compile.
- Unit tests cover the new config model.

### Phase 3: Core Monad EVM Factory

1. Add `MonadEvmNetwork`.
2. Add `MonadEvmFactory` or implement `FoundryEvmFactory` for the factory type provided by
   `alloy-monad-evm`.
3. Integrate Monad context, journal, gas params, and precompiles.
4. Adapt nested EVM support.
5. Adapt fork initialization.

Exit criteria:

- `foundry-evm-core` compiles.
- Minimal execution tests pass.

### Phase 4: Cheatcodes and Precompiles

1. Port Monad cheatcode address and dispatcher.
2. Port staking storage adapter.
3. Port reserve-balance precompile support.
4. Port `vm.setEvmVersion` Monad hardfork support.
5. Update cheatcode assets and Solidity interfaces.

Exit criteria:

- `foundry-cheatcodes` compiles.
- Monad staking cheatcode tests pass.

### Phase 5: Forge, Cast, Chisel, Script, Verify

1. Wire Monad network into Forge test runner.
2. Wire Monad into script simulation and broadcast paths.
3. Wire Monad into Cast call/run paths.
4. Wire Monad into Chisel executor.
5. Wire Monad into verification utilities.

Exit criteria:

- `forge`, `cast`, `chisel`, `script`, and `verify` crates compile.
- Focused CLI tests pass.

### Phase 6: Anvil

1. Add Monad CLI and config support.
2. Add Monad backend execution.
3. Add Monad fork detection.
4. Add Monad node info reporting.
5. Add transaction pool validation.
6. Add tests for local and fork mode.

Exit criteria:

- Anvil unit and integration tests compile and pass.
- Monad Anvil shell regressions pass.

### Phase 7: Traces, Docs, Installer, CI

1. Port trace decoding.
2. Update README and config docs.
3. Update `foundryup`.
4. Update release workflow.
5. Add CI jobs for Monad regressions.

Exit criteria:

- Docs describe v1.7-style config.
- Installer can install a test release artifact.
- CI matrix is ready for release.

## Test Plan

Run these as gates during the port. Some commands may need package selection adjustments as the
branch evolves.

### Formatting and Metadata

```sh
cargo fmt --all --check
cargo metadata
cargo tree -d
```

### Rust Checks

```sh
cargo check -p foundry-evm-hardforks --all-targets
cargo check -p foundry-evm-networks --all-targets
cargo check -p foundry-evm-core --all-targets
cargo check -p foundry-cheatcodes --all-targets
cargo check -p foundry-evm --all-targets
cargo check -p forge --all-targets
cargo check -p cast --all-targets
cargo check -p anvil --all-targets
cargo check -p chisel --all-targets
cargo check -p foundry-verify --all-targets
```

### Unit and Integration Tests

```sh
cargo test -p foundry-evm-hardforks --lib
cargo test -p foundry-evm-networks --lib
cargo test -p foundry-config --lib
cargo test -p foundry-evm-core --lib
cargo test -p foundry-cheatcodes --lib
cargo test -p foundry-evm --lib
cargo test -p anvil --lib
cargo test -p anvil --test it
cargo test -p forge --test cli
cargo test -p cast --test cli
```

### Monad Regression Tests

Restore or adapt these existing Monad regression gates from the old branch:

```sh
./script/forge/test_mip3.sh
scripts/run_monad_revm_state_tests.sh
./script/test/anvil/test_mip3_memory.sh
bash ./script/forge/test_mip4.sh
bash ./script/test/anvil/test_mip4_reserve_balance.sh
```

Additional focused checks:

```sh
forge test --match-contract MonadStakingTest -vv
forge test --match-contract StorageSlotStateTest -vv
forge test --match-contract LastCallGasDefaultTest -vv
anvil --monad --hardfork MonadNine
anvil --network monad --hardfork monad:MonadNine
```

### Upstream Regression Coverage

Run focused upstream v1.7 areas that are easy to break while adding Monad:

- Fuzz tests.
- Invariant tests.
- Trace tests.
- Script tests.
- Verify tests.
- Browser-wallet compile paths.
- Tempo compile and smoke tests.
- Optimism Anvil tests.

## Acceptance Criteria

The port is complete when all of the following are true:

1. The branch is based on upstream `v1.7.0`.
2. Monad dependencies are aligned with the upstream v1.7 dependency stack.
3. `network = "monad"` works in config.
4. `--network monad` works in CLI paths.
5. `--monad` remains available as a compatibility alias.
6. `hardfork = "monad:MonadNine"` works.
7. `monad_hardfork = "MonadNine"` still works or has a documented migration path with tests.
8. `MonadNine` is the default Monad hardfork unless intentionally changed.
9. Monad EVM execution is used by Forge, Cast, Chisel, Script, Verify, and Anvil when Monad is
   active.
10. Monad staking precompile behavior is correct.
11. Monad reserve-balance precompile behavior is correct.
12. Monad staking cheatcodes pass their Solidity tests.
13. Monad trace decoding works.
14. Anvil local Monad mode works.
15. Anvil forked Monad mode works.
16. Upstream Ethereum behavior remains intact.
17. Upstream Optimism behavior remains intact.
18. Upstream Tempo behavior remains intact.
19. Release and installer paths can produce and install Monad artifacts.
20. CI has a clear Monad regression gate.

## Risk Register

### Dependency Drift

Risk: `monad-revm` and `alloy-monad-evm` may not yet support `revm 38` and `alloy-evm 0.33.2`.

Mitigation:

- Resolve this before deeper Foundry edits.
- Prefer updating Monad crates rather than downgrading upstream v1.7 dependencies.
- Avoid duplicate dependency stacks.

### Foundry Precompile Bound

Risk: upstream Foundry v1.7 requires `FoundryEvmFactory` implementations to use
`PrecompilesMap`, while current `alloy-monad-evm` uses `MonadPrecompilesMap`.

Mitigation:

- Do not begin Foundry integration until `alloy-monad-evm::MonadEvmFactory` has
  `type Precompiles = PrecompilesMap`.
- Implement Monad-specific `PrecompileProvider<MonadContext<DB>> for PrecompilesMap` in the
  Monad crates, preserving staking and reserve-balance behavior.
- Treat a Foundry-side relaxation of the bound as a fallback only, because it would make a future
  upstream PR larger and harder to justify.

### Generic EVM Type Complexity

Risk: Monad context/journal/precompile types may not fit upstream `FoundryEvmFactory` cleanly.

Mitigation:

- Use Tempo's v1.7 implementation as the primary reference.
- Add Monad-specific adapter traits only where generic bounds cannot express the behavior.
- Keep type aliases local and explicit.

### Cheatcode Context Assumptions

Risk: Old Monad cheatcodes assume `MonadContext` globally, while v1.7 cheatcodes are generic.

Mitigation:

- Port cheatcodes after the core Monad EVM factory compiles.
- Add tests for standard cheatcodes under Monad.
- Add tests for Monad cheatcodes under nested execution.

### Anvil Execution Surface

Risk: Anvil has the largest operational surface: pool validation, RPC types, block execution,
forking, receipts, state dumps, and traces.

Mitigation:

- Port Anvil after core Forge execution works.
- Keep Anvil work in smaller slices.
- Use local Anvil smoke tests after each slice.

### Config Migration

Risk: Users may already rely on `monad_hardfork`.

Mitigation:

- Keep `monad_hardfork` for compatibility.
- Prefer warning/deprecation over removal.
- Document the v1.7-style replacement.

### Upstream Feature Regressions

Risk: Monad conditionals could break Tempo, Optimism, browser-wallet, or generic script paths.

Mitigation:

- Use network-family matching rather than boolean checks in shared paths.
- Keep upstream v1.7 tests active.
- Avoid replacing generic code with Monad-specialized code.

## Commit Strategy

Recommended commit groups:

1. `chore(monad): add v1.7 port work spec`
2. `chore(deps): add monad dependency stack for v1.7`
3. `feat(hardforks): add monad hardfork family`
4. `feat(networks): add monad network variant`
5. `feat(evm): add monad foundry evm factory`
6. `feat(cheatcodes): port monad staking cheatcodes`
7. `feat(precompiles): port monad staking and reserve balance decoding`
8. `feat(forge): wire monad execution`
9. `feat(cast): wire monad execution`
10. `feat(chisel): wire monad execution`
11. `feat(verify): wire monad verification behavior`
12. `feat(anvil): add monad execution mode`
13. `test(monad): port monad regression fixtures`
14. `docs(monad): update v1.7 configuration and install docs`
15. `ci(monad): add v1.7 monad release and regression jobs`

Each commit group should compile or have a clearly documented temporary reason it cannot compile.
Do not leave large mixed commits that combine dependency changes, config changes, EVM execution,
and Anvil behavior.

## Open Questions

1. Should default Monad behavior be encoded as a fork-specific build default, or should users
   always specify `network = "monad"` in new v1.7 projects?
2. Should `monad_hardfork` be retained indefinitely or deprecated after one release line?
3. Should Monad hardfork parsing accept lowercase forms like `monadnine`, or only canonical
   `MonadNine` and namespaced `monad:MonadNine`?
4. Should local `anvil_nodeInfo` expose `network = "monad"` in the same shape upstream uses for
   Tempo?
5. Is `MonadNext` intended for public use in this release, or should it be hidden/experimental?
6. Which Monad RPC endpoints are stable enough for CI fork tests?

## Release Readiness Checklist

- [ ] Dependency stack aligned.
- [ ] `Cargo.lock` reviewed.
- [ ] `cargo fmt --all --check` passes.
- [ ] Core Rust checks pass.
- [ ] Upstream focused tests pass.
- [ ] Monad Forge tests pass.
- [ ] Monad Anvil tests pass.
- [ ] Monad MIP-3 tests pass.
- [ ] Monad MIP-4 tests pass.
- [ ] Monad staking cheatcode tests pass.
- [ ] Monad trace decoding tests pass.
- [ ] Installer tested from a draft release.
- [ ] Release notes written.
- [ ] README updated.
- [ ] Config migration documented.
- [ ] Known issues documented.
