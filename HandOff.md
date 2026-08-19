# Task Handoff: Embedded Solar LSP Review Follow-up

## Objective

Assess the review comment on crates/forge/tests/cli/lsp.rs and remove tests that duplicate Solar's
workspace/remapping/flycheck coverage while retaining Forge-owned CLI and stdio boundary coverage.

## Current State

- Repository: /Users/yuhang/foundry, branch lsp_wrapper, HEAD 52f4843fa.
- Worktree changes: crates/forge/tests/cli/lsp.rs and this handoff only.
- Review conclusion: the comment is precise for the remapping and flycheck cases. Forge's
  crates/forge/src/cmd/lsp.rs forwards directly to solar_lsp::run_server_stdio, while pinned Solar
  already tests Foundry remapping/workspace loading and flycheck configuration, parsing, and
  lifecycle behavior.
- The default Forge flycheck subprocess test was removed under the review's requested scope. It
  was a unique cross-crate smoke test, but it asserted Solar's default flycheck path rather than a
  Forge-owned adapter contract.

## Changes

- Removed lsp_stdio_discovers_foundry_toml_remappings, lsp_stdio_discovers_remappings_txt,
  assert_remapping_definition, and lsp_stdio_runs_default_forge_flycheck.
- Replaced the threaded LspClient with direct batch JSON-RPC framing in
  lsp_stdio_handshake_uses_only_lsp_stdout; the test still covers both forge lsp and
  forge lsp --stdio, clean stdout, response ordering, and successful shutdown.
- Kept Forge-specific help, unsupported-global, and malformed-global-value tests.

## Verification

- cargo +nightly fmt --all -- --check passed.
- git diff --check passed.
- cargo test --offline --locked -p forge --test cli lsp:: -- --nocapture passed 4/4.
- cargo clippy --offline --locked -p forge --test cli -- -D warnings passed.
- Only existing linker/future-incompatibility warnings were emitted; no command failed.

## Next Action

No implementation work remains. Review the final diff and report the duplicate-coverage rationale
and verification results.
