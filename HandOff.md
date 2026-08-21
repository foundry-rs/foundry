# Task Handoff: Fix Duplicate Forge LSP Diagnostics

## Objective And Success Criteria

Update Foundry's coordinated Solar pin to a formal upstream revision that preserves the embeddable
`LaunchConfig` API and includes mutually exclusive push/pull diagnostic delivery. The same raw LSP
probe must change from duplicate push-plus-pull delivery to pull-only delivery, and the affected
Foundry code must compile and retain its lint and LSP behavior.

## Constraints And Decisions

- Repository: `/Users/yuhang/foundry`; branch `lsp_wrapper`; HEAD `669624928`.
- Preserve the completed `forge lsp` integration already committed on this branch.
- Move all four Solar `[patch.crates-io]` entries together; Cargo resolves 12 Solar packages from
  one git revision.
- Use formal Solar #1209 squash-merge `e2e626864495f4693d6897beee9d20f2491732cc`.
  Its tree exactly matches PR head `f1a08500d8913c5d4102bd8019a8e51a69a089c2`, and Solar's
  diagnostic compatibility fix `8755964b` is an ancestor.
- Keep Solar-owned protocol coverage upstream. Foundry retains its existing stdio handshake test;
  the host-level regression proof is the external raw-stdio probe in
  `/tmp/solar-diag-delivery-probe.rb`.
- Keep the current lockfile. Reproducing from clean HEAD with the narrow non-recursive command
  `cargo update -p solar-compiler --precise e2e626864495f4693d6897beee9d20f2491732cc`
  produces it byte-for-byte. No package identity, registry version, checksum, or package count
  changes; Cargo only rebinds dependency edges among versions already present in the lock.

## Verified Root Cause

- The old Solar pin `ba818eef89b1f5d683c482e2702b74772fc27b37` advertised pull diagnostics
  while also publishing the same diagnostic through `textDocument/publishDiagnostics`.
- `vscode-languageclient` keeps push and pull diagnostics in separate collections, so VS Code
  displayed the identical Solar item twice. One server process was running; flycheck diagnostics
  use source `forge-lint` and were not the duplicate.
- Solar #1159 fixes the protocol boundary by negotiating exactly one
  `DiagnosticDelivery::{Push, Pull}` mode. Pull requires both document-diagnostic and workspace
  diagnostic-refresh capabilities; clients lacking either capability continue to receive push.

## Completed Work

- Updated the four Solar patch entries and all 12 Solar lockfile source records to `e2e62686`.
- Migrated four Foundry lint call sites from removed Solar APIs
  `Gcx::{builtin_member,resolved_member,builtin_callee}` to the unified
  `Gcx::{resolved_builtin,resolved_expr}` queries, matching Solar's own migration in `6844b2d9`.
- Preserved existing documentation and Forge-owned LSP test scope; no new Foundry test was added
  for Solar internals. Updated the existing changelog entry to include the affected publishable
  `forge-lint` package.

## Changed Files

- `Cargo.toml`: coordinated Solar pin.
- `Cargo.lock`: reproducible Solar source/manifest resolution.
- `crates/lint/src/sol/high/function_selector_collision.rs`: new Solar expression-resolution API.
- `crates/lint/src/sol/med/ecrecover.rs`: new Solar builtin-resolution API.
- `.changelog/forge-lsp.md`: diagnostic-delivery note and `forge-lint` patch mapping.
- `HandOff.md`: rolling task state.

## Red And Green Protocol Results

Old pin, expected probe failure:

```json
{
  "push_count": 1,
  "pull_count": 1,
  "push_equals_pull": true
}
```

New pin, probe passed:

```json
{
  "diagnostic_provider": {
    "interFileDependencies": true,
    "workspaceDiagnostics": true,
    "workDoneProgress": true
  },
  "push_count": 0,
  "pull_count": 1,
  "push_equals_pull": false
}
```

## Verification

- Solar focused diagnostic-delivery tests on the source-identical PR-head tree: 6 passed,
  1007 skipped.
- `cargo check --locked -p forge-lint`: passed.
- `cargo build --locked -p forge --bin forge`: passed.
- `ruby /tmp/solar-diag-delivery-probe.rb /Users/yuhang/foundry/target/debug/forge`: passed with
  pull 1, push 0.
- `cargo test --locked -p forge --test ui -- Ecrecover`: passed, 97 fixtures selected and
  96 filtered out; both `Ecrecover.sol` and `FunctionSelectorCollision.sol` passed.
- `cargo nextest run --locked -p forge --test ui`: passed, 1/1 UI runner (all lint fixtures).
- `cargo test --locked -p forge --test cli lsp:: --no-fail-fast`: passed, 1/1.
- `cargo metadata --locked --no-deps --format-version 1`: passed.
- `cargo check --locked -p forge -p chisel -p solar --bins`: passed.
- `cargo +nightly fmt --all -- --check`: passed after formatting.
- `cargo clippy --locked -p forge-lint -p forge --all-targets`: passed.
- `git diff --check`: passed.

Cargo continues to emit pre-existing global-cache cleanup permission, macOS linker compact-unwind,
and future-incompatibility warnings. They do not fail any command and are unrelated to this diff.

## Remaining Work

No implementation or verification work remains. These changes belong on `0xKarl98/lsp_wrapper`.
