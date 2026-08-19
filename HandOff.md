# Task Handoff: Embedded Solar LSP Findings

## Objective

Apply and preserve the three confirmed findings for PR #16254 / branch `lsp_wrapper`: correct the
README/CLI contract, document the Solar flycheck dotenv boundary without weakening the security
policy, and harden/test malformed LSP argument pre-scanning.

## Review State

- Workspace: `/Users/yuhang/foundry`; branch `lsp_wrapper` at commit `35a099070`.
- Base: `origin/master` at `423ac0d40`.
- PR: `https://github.com/foundry-rs/foundry/pull/16254`, head matches local `35a099070`.
- Product diff from `origin/master`: 11 files, 811 insertions, and 4 deletions across four commits.
- Worktree changes are intentionally limited to `README.md`, `crates/forge/src/args.rs`,
  `crates/forge/tests/cli/lsp.rs`, `.changelog/forge-lsp.md`, and this handoff; preserve unrelated
  user changes.
- Applicable skills read: `karpathy-guidelines` and `rust-diagnose`.
- Confirmed code and tests reject unsupported global flags, including `--threads`/`--jobs`, and hide
  them from LSP help. The stale README claim that those flags are accepted is fixed in the current
  worktree.
- PR page is open and has no maintainer review conclusion; its description matches the intended
  embedded Solar LSP scope.
- Current focused verification passed: `cargo +nightly fmt --all -- --check`, `git diff --check`,
  `cargo check --offline --locked -p forge --all-targets`, and
  `cargo +nightly clippy --offline --locked -p forge --lib --tests -- -D warnings`.
- Current parser/unit coverage is 5/5 (`cargo test --offline --locked -p forge args::tests --lib`);
  Forge option coverage is 2/2 (`cargo test --offline --locked -p forge opts::tests --lib`); current
  LSP CLI coverage is 7/7 (`cargo test --offline --locked -p forge --test cli lsp::`).
- Before the hardening patch, `forge --color lsp` and `forge --threads lsp` could consume `lsp` as
  an option value, enter normal setup, and report an unapproved `.env` error before the CLI error.
  The pre-scan now performs a strict value-validation fallback only when no subcommand was found;
  valid option values followed by a real command remain outside the LSP path.
- Solar's default `forge-lint` flycheck probes/runs `forge lint --json` with null stdin. In a project
  containing an unapproved `.env`, that child is rejected by Foundry's dotenv policy, so core LSP
  analysis still starts but default `forge-lint` diagnostics are unavailable. This is an indirect
  Solar flycheck/security-policy limitation and needs an explicit safety decision before changing.
- No unrelated lockfile drift remains in the product diff; the added entries are `solar-lsp` and its
  transitive graph plus the feature-induced `tokio-util` edge.

## Confirmed Findings

1. **P2: README contradicted the implementation.** `README.md:82` said `--threads`/`--jobs`
   were accepted to size Tokio, but `crates/forge/src/args.rs` rejects Forge global options and
   tests require rejection. Decision: correct the README; keep the current rejection behavior.

2. **P2: default Forge flycheck is unavailable for unapproved `.env` projects.** Solar's default
   flycheck probes/runs `forge lint --json` as a child with null stdin. Foundry's dotenv approval
   path rejects that non-interactive child, so core Solar analysis can start while `forge-lint`
   diagnostics are absent. Decision: document the boundary and do not append `--allow-project-env`
   implicitly. Any opt-in must be explicit and trusted in flycheck configuration.

3. **Boundary: malformed values could be consumed as the `lsp` token.** Before hardening,
   `forge --color lsp` and `forge --threads lsp` could enter normal setup and report dotenv failure
   before the CLI error. Decision: after permissive parsing finds no subcommand, run a strict parse
   only for `InvalidValue`/`ValueValidation`; preserve normal commands whose option value happens to
   be `lsp`. Unit and CLI regressions cover space-separated `--color`, `--threads`, `--jobs`, and
   `-j` forms with an unapproved `.env`.

## Scope Map

- CLI parsing: `crates/forge/src/opts.rs`.
- pre-setup routing and dispatch: `crates/forge/src/args.rs`.
- wrapper: `crates/forge/src/cmd/lsp.rs`.
- dependency/feature changes: root and Forge `Cargo.toml`, `Cargo.lock`.
- behavior coverage: `crates/forge/tests/cli/lsp.rs` plus parser unit tests in `args.rs`.
- user-facing scope: `README.md`, `.changelog/forge-lsp.md`.

## Success Criteria

- `forge lsp` works without an external `solar` executable or Solar-specific setup.
- A normal Foundry user can start `forge lsp` and use Solar LSP features without needing to know
  Solar's standalone CLI or configure Solar-specific options.
- stdout remains an uncorrupted JSON-RPC channel and startup avoids unrelated Forge setup.
- CLI grammar and supported global arguments are coherent; malformed invocations fail predictably.
- embedding Solar does not introduce avoidable dependency, MSRV, packaging, or behavior changes.
- `forge lsp --help` hides rejected global options.
- `--threads`/`--jobs` are treated like other unrelated Forge globals: hidden from LSP help and
  rejected when supplied.
- normal Forge command help and LSP runtime behavior remain unchanged.

## Completed

- Added a dedicated built Clap command tree for LSP parsing in `crates/forge/src/args.rs`.
- After Clap propagates global arguments into subcommands, hidden every global option on the LSP
  help surface; hidden arguments remain parseable so the existing rejection diagnostic still runs.
- Restored the six unrelated `Cargo.lock` dependency edges to the versions selected by the base
  lockfile: `bon-macros`/`darling`, `foundry-compilers`/`itertools`, `once_map`/`hashbrown`,
  `prost-derive`/`itertools`, `solar-interface`/`itertools`, and `solar-parse`/`itertools`.
- Removed the redundant `lsp` token pre-scan in `is_lsp_invocation` while preserving the public
  `run_command` LSP dispatch behavior for direct callers.
- Kept normal Forge parsing on `Forge::parse()` and normal help unchanged.
- Added unit coverage for the rendered LSP help and CLI coverage for actual `forge lsp --help`
  output.
- Verification passed: `cargo +nightly fmt --all -- --check`, `cargo check --offline --locked -p forge --all-targets`,
  `cargo +nightly clippy --offline --locked -p forge --lib --tests -- -D warnings`,
  `cargo test --offline --locked -p forge args::tests --lib` (5 tests),
  `cargo test --offline --locked -p forge opts::tests --lib` (2 tests),
  `cargo test --offline --locked -p forge --test cli lsp::` (7 tests), and `git diff --check`.
- Manual `target/debug/forge lsp --help` output contains Solar's description and the help option;
  Solar's hidden-by-default `--stdio` plus `--threads`/`--jobs` are absent.
- Manual `forge lsp --threads 2` exits with the unsupported-global-option diagnostic and empty
  stdout; normal `forge --help` still exposes Forge's global options.
- Cargo emitted only existing future-incompatibility, linker, and cache-cleanup permission warnings;
  no verification command failed.
- Cleanup commit `7105b3074` (`chore(forge): clean up lsp wrapper`) was pushed successfully; the
  target branch now resolves to `7105b30740657d08782b9bec5a4c0abc938fede7`.
- Restoring the public `run_command` LSP arm after review is verified, committed as `35a099070`, and
  pushed to the target branch.
- Corrected `README.md`: `forge lsp` accepts only LSP-specific options and rejects Forge globals,
  including `--threads`/`--jobs`.
- Documented the Solar default flycheck boundary: Solar probes/runs a separate `forge lint --json`
  child with null stdin; an unapproved project `.env` therefore suppresses `forge-lint` diagnostics
  while core LSP analysis can continue. Do not append `--allow-project-env` implicitly; an explicit,
  trusted flycheck configuration is the opt-in path.
- Hardened `crates/forge/src/args.rs` for space-separated malformed values and added unit coverage.
  Added CLI regression cases for `--color lsp`, `--threads lsp`, `--jobs lsp`, and `-j lsp` in a
  project containing an unapproved `.env`, asserting the CLI `invalid value` error wins.

## Next Actions

- No required implementation work remains. Keep the explicit dotenv opt-in question open for a
  separate security/product decision; do not add `--allow-project-env` implicitly to the wrapper.

## Prior Review Notes

- Confirmed lockfile-only version drift unrelated to the new `solar-lsp` graph:
  `bon-macros` switched `darling 0.20.11 -> 0.23.0`, `foundry-compilers` switched
  `itertools 0.13.0 -> 0.15.0`, `once_map` switched `hashbrown 0.16.1 -> 0.17.1`, and
  `prost-derive 0.14.4` switched `itertools 0.12.1 -> 0.14.0`.
- Existing Solar packages (`solar-interface` and `solar-parse`) also switched from
  `itertools 0.12.1` to `0.14.0`; this is compatible with Solar's declared range but has not yet
  been proven necessary for LSP.
- New `async-lsp`, `crop`, `lsp-types`, `serde_repr`, `str_indices`, and `waitpid-any` entries are
  transitive dependencies of `solar-lsp` and currently appear necessary.
- No removable product-code or test-code block was confirmed. The early LSP parser and the
  `solar` `clap` feature remain behaviorally necessary for the tested invocation paths.
- Focused checks passed: `cargo test -p forge args::tests --lib` (3 tests) and
  `cargo test -p forge --test cli lsp::` (7 tests).
- Confirmed UX finding: `forge lsp --help` exposes inherited global flags that
  `reject_unsupported_lsp_globals` rejects at `crates/forge/src/args.rs:71-95`; this is misleading
  help and should be fixed or explicitly hidden.
- Solar dependency source was checked at the pinned revision `a38f69e4f2e2267c549555a980aef6b0b1f249eb`.
  `solar_config::LspArgs` contains only hidden, ignored `--stdio`; `solar_lsp::run_server_stdio`
  ignores the args entirely. Solar's CLI drops top-level `CompileOpts` in the `Lsp` arm, and the
  LSP workspace loader reads only `[profile.default]` fields from `foundry.toml`.
- Therefore `--profile`, Forge output/logging flags, local-compiler approval, dotenv approval, and
  `--threads`/`--jobs` have no Solar LSP semantics and the rejection path is justified. The latter
  two were previously retained as a Forge-wrapper runtime extension, but the issue scope does not
  require exposing that extension.
- Reproduced `target/debug/forge lsp --help`: it lists rejected inherited globals (`--profile`,
  `--color`, `--json`, `--md`, `--quiet`, `--verbosity`, compiler/project options), confirming the
  help mismatch. `forge lsp --profile ci`, `--quiet`, `--color never`, `--json`, and approval flags
  fail before emitting stdout; `--threads 2` reaches the LSP runtime when a valid client is attached.
- Re-run verification on this worktree: `cargo test -p forge args::tests --lib` passed 3/3 and
  `cargo test -p forge --test cli lsp::` passed 7/7. `git diff --check origin/master...HEAD` is clean.
- Review initially flagged the public `run_command` LSP arm as a possible invariant, but direct
  callers can rely on its existing behavior; the arm is retained after verification.
- A temporary lockfile with all six unrelated edges restored to base versions passed
  `cargo check --offline --locked -p forge --lib`; this proves the six lockfile upgrades are
  removable while preserving the new LSP graph.
- Solar audit also checked current `main`; the relevant CLI/LSP argument semantics are unchanged
  from the pinned revision. Strict Solar compatibility would reject `--threads` too, since Solar
  drops its root `CompileOpts` on the LSP path; the branch's exception is a deliberate Forge wrapper
  runtime control through `GlobalArgs::block_on`.
- Clarified the `.env` relationship: it is not part of Solar `LspArgs` or LSP initialization. It
  affects Solar's optional default `forge-lint` flycheck, which probes/runs a child `forge lint
  --json` with null stdin. In a temporary Git-rooted Foundry project, that child exits with
  `refusing to load unapproved project dotenv`; adding `--allow-project-env` makes it run. This is
  an indirect flycheck limitation, not evidence that `LspArgs` should carry Foundry globals.

<!-- codex-precompact:start -->
## Automatic Checkpoint

- Updated: 2026-08-19T15:29:31+00:00.
- Workspace: `/Users/yuhang/foundry`; branch `lsp_wrapper`; HEAD `35a099070`.
- Current worktree edits: `.changelog/forge-lsp.md`, `README.md`, `crates/forge/src/args.rs`,
  `crates/forge/tests/cli/lsp.rs`, and untracked `HandOff.md`.
- Final focused tests/checks passed; exact commands and counts are recorded above.
<!-- codex-precompact:end -->
