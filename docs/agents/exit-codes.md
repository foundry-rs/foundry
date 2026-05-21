# Foundry exit-code table

This document defines the canonical exit-code contract for Foundry binaries.
Exit codes are part of the agent contract — agents may switch on them.

The table below reflects the target contract. Until the `ExitCode` enum is
in place, binaries continue to emit `0` on success and a non-zero code on
failure without further guarantees.

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

Under `--machine`, the binary always emits a JSON envelope on stdout when
exiting with a non-zero code, including for parse and usage errors — these
are emitted as `JsonEnvelope::error` with the appropriate diagnostic code.
Help and version requests are not failures: they exit `0` and are emitted as
a success envelope wrapping the rendered text.
