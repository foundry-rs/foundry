# Output channels (stdout / stderr) — contract

This document defines the rules every Foundry CLI command (`forge`, `cast`, `anvil`,
`chisel`, `script`, `verify`) must follow when writing to standard output and standard
error.

> **Status:** The macros, helpers, and routing primitives described here are
> implemented. The per-command stdout table at the bottom is the **target
> contract** that downstream migration PRs will bring each command into
> compliance with; it does **not** yet describe today's behavior for every
> command.

## The contract

> **stdout** is the command's primary result and nothing else. In text mode it is one
> canonical value (or one record per line, tab-separated for multi-column output).
> In `--json` mode it is a single JSON document.
>
> **stderr** is every other byte the command emits: warnings, errors, progress
> indicators, status prose, prompts, banners, ABI dumps, verification chatter.

## Corollaries

- `--json` only changes stdout's *format*, never its *channel cleanliness*.
- `--quiet` suppresses stderr diagnostics and progress. The target contract is that
  it **never** alters stdout content; today `sh_print!`/`sh_println!` are still
  suppressed by `--quiet` and that bypass will be flipped on once the major prose
  stdout call sites in `forge`/`script` have been migrated to `sh_status!`.
  `sh_err!` is the documented exception: fatal errors are always emitted on stderr.
- `-vvv` adds stderr verbosity. It must never alter stdout content.
- A command that has no primary result (e.g. `forge install`) writes nothing to
  stdout by default.
- Prompts are diagnostics: write the question to stderr, read the answer from stdin.

The acceptance criterion this enables: an agent or shell script can run any
command, discard stderr, and trust that stdout contains only the documented
machine-readable result.

```sh
forge create … 2>/dev/null              # → contract address only
forge test --json 2>/dev/null | jq …    # → valid JSON, always
cast call … 2>/dev/null | xargs …       # → return value only
```

## How to write code that follows the contract

Use the `sh_*` macros in [`foundry_common::io`]; do not call `println!` /
`eprintln!` directly. A workspace-wide clippy `disallowed-macros` lint
(see [`clippy.toml`](../../clippy.toml)) forbids the four `std::print*` /
`std::eprint*` macros everywhere; the `sh_*` macros expand to `write!` calls on
the global `Shell`, so they are not affected by the lint.

| Macro            | Channel | Suppressed by `--quiet`            | Use for                                                            |
| ---------------- | ------- | ---------------------------------- | ------------------------------------------------------------------ |
| `sh_println!`    | stdout  | yes (target: **no**, see above)    | The command's primary machine-readable result.                     |
| `sh_print!`      | stdout  | yes (target: **no**, see above)    | Same as `sh_println!`, no trailing newline.                        |
| `sh_status!`     | stderr  | yes                                | Status prose ("Compiling…", "Deploying contract…").                |
| `sh_progress!`   | stderr  | yes (also no-op when stderr ≠ tty) | Spinner/progress-bar style transient updates.                      |
| `sh_warn!`       | stderr  | yes                                | Recoverable problems. Adds a "Warning:" prefix.                    |
| `sh_err!`        | stderr  | **no**                             | Errors. Adds an "Error:" prefix.                                   |
| `sh_eprintln!`   | stderr  | yes                                | Escape hatch for raw stderr text that doesn't fit the above.       |
| `sh_eprint!`     | stderr  | yes                                | Same as `sh_eprintln!`, no trailing newline.                       |
| `prompt!`        | stderr (question) + stdin (answer) | yes (question is `sh_eprint!`) | Interactive question/answer.                          |

The compilation reporter in `foundry_common::compile` writes its spinner to
stderr in TTY mode (see [`SpinnerReporter`](../../crates/common/src/term.rs)).
The non-TTY fallback still writes to stdout today; flipping it to stderr will
shift many existing snapshot tests and is part of the per-command migration
backlog. Calling `sh_progress!` directly is also acceptable for one-off
progress lines.

## Decision rule for any `sh_println!` call site

For each `sh_println!` call you write or review, ask:

1. **Is this the canonical primary result of the command?**
   - Yes → keep `sh_println!` (stdout).
   - No → use `sh_status!`, `sh_warn!`, `sh_err!`, or `sh_eprintln!` (stderr).
2. **Does this line mix labels with data** (e.g. `"Deployer: 0x…"`)?
   - The label is prose → move the *whole line* to `sh_status!` (stderr).
   - The data alone belongs on stdout → emit just the value (`sh_println!`).
   - In `--json` mode, both belong inside the single JSON document on stdout.

## Per-command stdout contract (target)

This table is the **target contract** that migration PRs will bring each
command into compliance with. It is the source of truth for what each command's
stdout *will* contain after migration, not necessarily what it contains today.
Each row's status is one of:

- `migrated` — current behavior matches this contract.
- `todo` — current behavior does not match yet; a follow-up PR is needed.

### `cast`

| Command                | Text mode stdout                                     | `--json` stdout                                                | Status |
| ---------------------- | ---------------------------------------------------- | -------------------------------------------------------------- | ------ |
| `cast call`            | Return value (hex / decoded)                         | JSON of return value                                           | todo   |
| `cast send`            | Tx hash                                              | JSON of receipt or `{ "hash": "0x…" }`                         | todo   |
| `cast estimate`        | Gas estimate (decimal)                               | JSON `{ "gas": "…" }`                                          | todo   |
| `cast rpc`             | RPC result (JSON)                                    | JSON                                                           | todo   |
| `cast storage`         | Single slot value                                    | JSON of layout                                                 | todo   |
| `cast logs`            | One log per line                                     | JSON array                                                     | todo   |
| `cast run`             | Trace / decoded output                               | JSON                                                           | todo   |
| `cast trace`           | Trace                                                | JSON trace                                                     | todo   |
| `cast wallet new`      | Address                                              | JSON `{ "address": "…", "private_key": "…" (only with explicit flag) }` | todo |
| `cast wallet sign`     | Signature                                            | JSON                                                           | todo   |
| `cast erc20 balance`   | Balance (decimal)                                    | JSON string                                                    | todo   |
| `cast access-list`     | Access list                                          | JSON                                                           | todo   |
| `cast da-estimate`     | Gas estimate                                         | JSON                                                           | todo   |
| `cast find-block`      | Block number                                         | JSON                                                           | todo   |
| `cast mktx`            | Signed RLP                                           | JSON                                                           | todo   |
| `cast batch-send`      | One tx hash per line                                 | JSON array                                                     | todo   |

### `forge`

| Command                | Text mode stdout                                     | `--json` stdout                            | Status |
| ---------------------- | ---------------------------------------------------- | ------------------------------------------ | ------ |
| `forge build`          | (empty)                                              | JSON build output                          | todo   |
| `forge test`           | (empty; exit code = pass/fail)                       | JSON test results, JUnit XML with `--junit`| todo   |
| `forge create`         | Deployed address                                     | JSON `{ "address": "…", "tx_hash": "…", … }` | todo |
| `forge inspect <field>`| Just that field's value                              | JSON of that field                         | todo   |
| `forge install`        | (empty)                                              | (empty)                                    | todo   |
| `forge init`           | (empty)                                              | (empty)                                    | todo   |
| `forge update`         | (empty)                                              | (empty)                                    | todo   |
| `forge remove`         | (empty)                                              | (empty)                                    | todo   |
| `forge clone`          | (empty)                                              | (empty)                                    | todo   |
| `forge bind`           | (empty)                                              | (empty)                                    | todo   |
| `forge bind-json`      | (empty) or generated path                            | JSON                                       | todo   |
| `forge flatten`        | Flattened source                                     | n/a                                        | todo   |
| `forge fmt`            | (empty) or formatted source with `--check`           | n/a                                        | todo   |
| `forge tree`           | Dependency tree                                      | JSON                                       | todo   |
| `forge config`         | Config TOML                                          | JSON config                                | todo   |
| `forge selectors`      | Selectors output                                     | JSON                                       | todo   |
| `forge eip712`         | (empty)                                              | JSON of types                              | todo   |
| `forge geiger`         | Findings                                             | JSON                                       | todo   |
| `forge lint`           | (empty; findings on stderr/exit code)                | JSON findings                              | todo   |
| `forge snapshot`       | Snapshot file content / diff                         | JSON                                       | todo   |
| `forge coverage`       | Coverage table or report                             | JSON / LCOV / etc. via `--report`          | todo   |
| `forge cache`          | (empty) or paths                                     | JSON                                       | todo   |
| `forge doc`            | (empty)                                              | n/a                                        | todo   |
| `forge generate`       | (empty) or generated path                            | n/a                                        | todo   |
| `forge soldeer`        | (empty)                                              | n/a                                        | todo   |
| `forge remappings`     | One remapping per line                               | n/a                                        | todo   |
| `forge compiler`       | Compiler info                                        | JSON                                       | todo   |
| `forge verify-contract`| Verification GUID / URL                              | JSON                                       | todo   |

### `anvil`, `chisel`, `script`

| Command       | Text mode stdout                              | `--json` stdout       | Status |
| ------------- | --------------------------------------------- | --------------------- | ------ |
| `anvil`       | Banner, accounts, RPC URL on stderr           | n/a                   | todo   |
| `chisel`      | REPL output                                   | n/a                   | todo   |
| `forge script`| Simulation/broadcast result                   | JSON                  | todo   |

Commands not listed here have not been classified yet — please open an issue or
PR before relying on their stdout format.

## References

- Macros: [`crates/common/src/io/macros.rs`](../../crates/common/src/io/macros.rs)
- Shell wrapper: [`crates/common/src/io/shell.rs`](../../crates/common/src/io/shell.rs)
- Spinner / progress: [`crates/common/src/term.rs`](../../crates/common/src/term.rs)
- Compilation reporter: [`crates/common/src/compile.rs`](../../crates/common/src/compile.rs)
- Lint configuration: [`clippy.toml`](../../clippy.toml)
