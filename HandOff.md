# Task Handoff: Embedded Solar LSP Setup Convergence

## Objective And Success Criteria

Finish the `forge lsp` integration so it uses Solar's embeddable launch API and follows Forge's
normal CLI control flow without any LSP-specific setup or parsing path.

Success means:

- `Forge` is type-parsed exactly once, followed by `GlobalArgs::init` and `run_command`.
- `lsp` uses the same process-wide environment, handler, tracing, shell, compiler approval, and
  global-option setup as every other Forge subcommand, while skipping project dotenv loading so
  its stdio transport cannot consume a prompt.
- The LSP path has no bespoke pre-parser, hidden-global command, or separate dispatch branch.
- Foundry injects its current executable as Solar's default Forge path.
- Forge retains one focused LSP handshake/stdout integration test; Solar owns launch internals.

## Constraints And Decisions

- Mattsse's review guidance is the primary design constraint and must survive session changes:
  keep the regular Forge path (`environment/setup -> parse Forge enum once -> GlobalArgs::init ->
  run_command -> LSP`), remove the `is_lsp_invocation`/`parse_lsp_args`/`run_lsp` setup bypass,
  and introduce the missing Solar launch-config boundary so Forge keeps
  `ForgeSubcommand::Lsp(LspArgs)` while `LspArgs` converts into `solar_lsp::LaunchConfig` before
  `solar_lsp::launch(config)`. Review source: PR #16254 discussion `r3813986870` plus Mattsse's
  follow-up message supplied by the user on 2026-08-21.
- Preserve normal commands' dotenv-before-strict-parse behavior. Reuse the existing permissive clap
  pass through a small command-aware predicate; do not add a second parser or dispatch path.
- Treat Forge global flags normally for `lsp`; do not hide or reject them in a parallel grammar.
- Keep Forge tests at the host boundary; Solar owns workspace, remapping, and flycheck internals.
- Pin Solar to `ba818eef`, the two-commit backport directly on Foundry's existing Solar revision.
  It exposes the same public launch contract as merged Solar PR #1209 without pulling 114 unrelated
  Solar commits and roughly 300 changed upstream files into this PR.
- Preserve unrelated user changes. Before this implementation only this handoff was modified.

## Verified Facts

- Branch `lsp_wrapper`, HEAD `3521babd96244ef5a04c8ba6b34b97d2621af777`.
- The branch at HEAD has `is_lsp_invocation`, `lsp_command`, `parse_lsp_args`,
  `reject_unsupported_lsp_globals`, and `run_lsp` branches around normal setup; the working diff
  removes all of them.
- `common_setup<C>` already performs a permissive clap pass before `load_dotenv`.
- Solar PR #1209 merged and exposes `solar_lsp::launch(LaunchConfig)` with a host-provided default
  Forge executable path. The two-commit PR #1205 head (`ba818eef`) is based directly on Foundry's
  existing Solar pin and has the same public contract; it is publicly fetchable at
  `refs/pull/1205/head`. The reviewed PR #1209 head is `f1a08500` and its merge is `e2e62686`.
- Cargo cannot move only `solar-lsp`: Foundry's unified Solar `[patch.crates-io]` source makes all
  12 Solar packages resolve at one revision. `Cargo.lock` therefore changes all 12 source entries,
  but no registry dependency versions.
- Reviewers explicitly objected to the bespoke setup bypass and its divergence from
  `run_command`; another review requested retaining only Forge-owned LSP integration coverage.

## Work State

- Completed: mapped boundaries; refactored Forge to regular process setup, one typed parse, and
  normal dispatch; added command-aware dotenv isolation without a second parser; removed bespoke
  global-option behavior; adopted `LaunchConfig` with `current_exe()`;
  pinned the narrow Solar backport; reduced coverage to Forge's handshake/stdout boundary; aligned
  README and changelog.
- Completed: formatting, focused tests, Forge check, default-feature clippy, diff checks, and final
  source review.
- The malformed `lsp`-as-global-value case intentionally follows the single normal parse path: a
  strict second pre-parser was not restored because Mattsse's guidance removes the bespoke LSP
  setup/parser branch. Such an invocation is a malformed global command, not a parsed LSP command.
- `--all-features` clippy could not reach Rust linting because the host Swift SDK/compiler versions
  disagree while building `foundry-wallets` Touch ID support; default-feature clippy passed.

## Changed Files

- `Cargo.toml`, `Cargo.lock`: move the coordinated Solar source to `ba818eef`; lockfile changes are
  source-only.
- `crates/cli/src/utils/mod.rs`, `crates/forge/src/args.rs`: reuse the permissive setup parse to
  skip only project dotenv loading for LSP while preserving all other setup.
- `crates/forge/src/args.rs`: removed the parallel LSP grammar/dispatch and restored normal flow.
- `crates/forge/src/cmd/lsp.rs`: converts `LspArgs` to `LaunchConfig`, injects `current_exe()`, and
  calls `solar_lsp::launch`.
- `crates/forge/tests/cli/lsp.rs`: retains only Forge's handshake/stdout boundary coverage.
- `README.md`, `.changelog/forge-lsp.md`: describe normal CLI flow and default Forge injection.
- `HandOff.md`: refreshed rolling task state.

## Verification

- `cargo metadata --locked --no-deps --format-version 1`: passed after the Solar source update.
- `cargo +nightly fmt --all -- --check`: passed after the final implementation.
- `git diff --check`: passed after implementation and lockfile convergence.
- `cargo test --locked -p forge --test cli lsp:: --no-fail-fast`: passed, 1/1.
- `cargo test --locked -p forge --lib opts::tests --no-fail-fast`: passed, 2/2.
- `cargo test --locked -p foundry-cli --lib utils --no-fail-fast`: passed, 19/19.
- Cargo emitted existing cache-cleanup permission, linker, and future-incompatibility warnings.
- `cargo check --locked -p forge --all-targets`: passed.
- `cargo clippy --locked -p forge --all-targets`: passed.
- `cargo clippy --locked -p forge --all-targets --all-features`: blocked before linting by the host
  SwiftBridging/Swift SDK mismatch in `foundry-wallets` Touch ID support.
- `git diff --check`: passed.

## Next Actions

1. No implementation work remains for this handoff. Preserve the current diff and report the
   verification results; do not reintroduce the removed bespoke LSP parser/setup path.

<!-- codex-precompact:start -->
## Automatic Pre-Compaction Checkpoint

- Updated: 2026-08-21T09:32:24+00:00
- Session: 01a0237b-37df-7ee0-8b2e-7a7b94e6578f
- Turn: 01a0238a-e98e-7c63-af95-9d82567d4e83
- Model: gpt-5.6-sol
- Trigger: auto
- Workspace: /Users/yuhang/foundry
- Branch: lsp_wrapper
- HEAD: 3521babd9

### Git Status

```text
M .changelog/forge-lsp.md
 M Cargo.lock
 M Cargo.toml
 M HandOff.md
 M README.md
 M crates/cli/src/utils/mod.rs
 M crates/forge/src/args.rs
 M crates/forge/src/cmd/lsp.rs
 M crates/forge/tests/cli/lsp.rs
```

### Unstaged Diff Stat

```text
.changelog/forge-lsp.md       |   4 +-
 Cargo.lock                    |  24 ++---
 Cargo.toml                    |   8 +-
 HandOff.md                    | 108 ++++++++++++++++------
 README.md                     |  21 ++---
 crates/cli/src/utils/mod.rs   |  26 ++++--
 crates/forge/src/args.rs      | 210 +-----------------------------------------
 crates/forge/src/cmd/lsp.rs   |   4 +-
 crates/forge/tests/cli/lsp.rs |  80 ----------------
 9 files changed, 133 insertions(+), 352 deletions(-)
```

### Staged Diff Stat

```text
(none or unavailable)
```

This generated block is a mechanical fallback. The semantic sections above must
be maintained during the task so that decisions and exact next actions survive.
<!-- codex-precompact:end -->
