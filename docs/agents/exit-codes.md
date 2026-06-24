# Foundry exit-code table

This document defines the canonical exit-code contract for Foundry binaries.
Exit codes are part of the agent contract — agents may switch on them.

The table below is the canonical contract. The [`ExitCode`] enum encodes
it for use in code. Adoption is incremental: commands return the
appropriate variant as they migrate; until then, callers should expect a
non-zero exit but may not get the specific code listed here.

[`ExitCode`]: ../../crates/cli/src/exit_code.rs

| Code | Name             | Meaning                                                                        |
| ---- | ---------------- | ------------------------------------------------------------------------------ |
| `0`  | `Success`        | Command completed successfully                                                 |
| `1`  | `GenericError`   | Unclassified failure                                                           |
| `2`  | `Usage`          | Argument parse error, missing subcommand, invalid flag combination             |
| `3`  | `Config`         | Foundry config invalid or missing required value                               |
| `4`  | `Build`          | Compilation, linking, or artifact generation failed                            |
| `5`  | `TestFailure`    | Tests ran but at least one failed (distinct from a build/setup failure)        |
| `6`  | `Network`        | RPC, HTTP, or chain-connectivity failure                                       |
| `7`  | `User`           | Authentication, authorization, or wallet/key-related failure                   |
| `8`  | `Interrupted`    | Command terminated by `SIGINT` / `SIGTERM`                                     |

Codes outside this set are reserved. Commands MAY document additional
command-specific codes in `CommandInfo.exit_codes` (introspection); those
codes MUST NOT collide with this global table.

## Mapping rules

- A failure to parse `--json` / `--machine` output flags themselves is `Usage`.
- A failure to load `foundry.toml` is `Config`.
- A test that compiled and ran but reverted is `TestFailure`, not `Build`.
- A network timeout during `cast call` is `Network`.
- A signed-message rejection or missing key is `User`.
- A build failure during `forge test` setup is `Build`, not `TestFailure`.

## Machine-mode interaction

Under `--machine`, the CLI runtime guarantees structured envelopes for
**pre-command exits** (parse errors, missing subcommand, invalid flag
combination, `--help`, `--version`):

- parse / usage failures exit `2` and emit `JsonEnvelope::error` with
  diagnostic code `cli.usage.invalid`
- `--help` / `--version` exit `0` and emit `JsonEnvelope::success`
  wrapping the rendered text (schemas `foundry:cli.help@v1` /
  `foundry:cli.version@v1`; see [`spec.md`](./spec.md) §10)

Command-local exit-code and envelope adoption is incremental. Each
command migrates to its canonical [`ExitCode`] and to envelope output
per its declared [`output_mode`](./spec.md#4-output-modes) as part of
follow-up work; until a command opts in, it exits `1` on failure (the
existing `main` fallback) and does not emit an envelope. The two
exit-code adoptions wired in the runtime layer today are `forge test`
failure → `TestFailure` (5) and `anvil` signal exit → `Interrupted` (8)
under `--machine`.
