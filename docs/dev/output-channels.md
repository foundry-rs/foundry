# Output channels (stdout / stderr) — contract

This document defines the rules every Foundry CLI command (`forge`, `cast`, `anvil`,
`chisel`, `script`, `verify`) must follow when writing to standard output and standard
error.

## The contract

> **stdout** is the command's primary result and nothing else. In text mode it is one
> canonical value (or one record per line, tab-separated for multi-column output).
> In `--json` mode it is a single JSON document.
>
> **stderr** is every other byte the command emits: warnings, errors, progress
> indicators, status prose, prompts, banners, ABI dumps, verification chatter.

## Corollaries

- `--json` only changes stdout's *format*, never its *channel cleanliness*.
- `--quiet` suppresses stderr diagnostics. It must never alter stdout content.
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
`eprintln!` directly (a clippy lint enforces this for command code).

| Macro            | Channel | Suppressed by `--quiet` | Use for                                                            |
| ---------------- | ------- | ----------------------- | ------------------------------------------------------------------ |
| `sh_println!`    | stdout  | yes                     | The command's primary machine-readable result.                     |
| `sh_print!`      | stdout  | yes                     | Same as `sh_println!`, no trailing newline.                        |
| `sh_status!`     | stderr  | yes                     | Status prose ("Compiling…", "Deploying contract…").                |
| `sh_progress!`   | stderr  | yes (and on non-tty)    | Spinner/progress-bar style transient updates.                      |
| `sh_warn!`       | stderr  | yes                     | Recoverable problems. Adds a "Warning:" prefix.                    |
| `sh_err!`        | stderr  | **no**                  | Errors. Adds an "Error:" prefix.                                   |
| `sh_eprintln!`   | stderr  | yes                     | Escape hatch for raw stderr text that doesn't fit the above.       |
| `sh_eprint!`     | stderr  | yes                     | Same as `sh_eprintln!`, no trailing newline.                       |
| `prompt!`        | stderr (question) + stdin (answer) | n/a | Interactive question/answer.            |

The spinner in `foundry_common::term` writes to stderr automatically. Calling
`sh_progress!` directly is also acceptable for one-off progress lines.

## Decision rule for any `sh_println!` call site

For each `sh_println!` call you write or review, ask:

1. **Is this the canonical primary result of the command?**
   - Yes → keep `sh_println!` (stdout).
   - No → use `sh_status!`, `sh_warn!`, `sh_err!`, or `sh_eprintln!` (stderr).
2. **Does this line mix labels with data** (e.g. `"Deployer: 0x…"`)?
   - The label is prose → move the *whole line* to `sh_status!` (stderr).
   - The data alone belongs on stdout → emit just the value (`sh_println!`).
   - In `--json` mode, both belong inside the single JSON document on stdout.

## Per-command stdout contract

This table is the source of truth for what each command may emit on stdout.

### `cast`

| Command                | Text mode stdout                                     | `--json` stdout                                                |
| ---------------------- | ---------------------------------------------------- | -------------------------------------------------------------- |
| `cast call`            | Return value (hex / decoded)                         | JSON of return value                                           |
| `cast send`            | Tx hash                                              | JSON of receipt or `{ "hash": "0x…" }`                         |
| `cast estimate`        | Gas estimate (decimal)                               | JSON `{ "gas": "…" }`                                          |
| `cast rpc`             | RPC result (JSON)                                    | JSON                                                           |
| `cast storage`         | Single slot value                                    | JSON of layout                                                 |
| `cast logs`            | One log per line                                     | JSON array                                                     |
| `cast run`             | Trace / decoded output                               | JSON                                                           |
| `cast trace`           | Trace                                                | JSON trace                                                     |
| `cast wallet new`      | Address                                              | JSON `{ "address": "…", "private_key": "…" (only with explicit flag) }` |
| `cast wallet sign`     | Signature                                            | JSON                                                           |
| `cast erc20 balance`   | Balance (decimal)                                    | JSON string                                                    |
| `cast access-list`     | Access list                                          | JSON                                                           |
| `cast da-estimate`     | Gas estimate                                         | JSON                                                           |
| `cast find-block`      | Block number                                         | JSON                                                           |
| `cast mktx`            | Signed RLP                                           | JSON                                                           |
| `cast batch-send`      | One tx hash per line                                 | JSON array                                                     |

### `forge`

| Command                | Text mode stdout                                     | `--json` stdout                            |
| ---------------------- | ---------------------------------------------------- | ------------------------------------------ |
| `forge build`          | (empty)                                              | JSON build output                          |
| `forge test`           | (empty; exit code = pass/fail)                       | JSON test results, JUnit XML with `--junit`|
| `forge create`         | Deployed address                                     | JSON `{ "address": "…", "tx_hash": "…", … }` |
| `forge inspect <field>`| Just that field's value                              | JSON of that field                         |
| `forge install`        | (empty)                                              | (empty)                                    |
| `forge init`           | (empty)                                              | (empty)                                    |
| `forge update`         | (empty)                                              | (empty)                                    |
| `forge remove`         | (empty)                                              | (empty)                                    |
| `forge clone`          | (empty)                                              | (empty)                                    |
| `forge bind`           | (empty)                                              | (empty)                                    |
| `forge bind-json`      | (empty) or generated path                            | JSON                                       |
| `forge flatten`        | Flattened source                                     | n/a                                        |
| `forge fmt`            | (empty) or formatted source with `--check`           | n/a                                        |
| `forge tree`           | Dependency tree                                      | JSON                                       |
| `forge config`         | Config TOML                                          | JSON config                                |
| `forge selectors`      | Selectors output                                     | JSON                                       |
| `forge eip712`         | (empty)                                              | JSON of types                              |
| `forge geiger`         | Findings                                             | JSON                                       |
| `forge lint`           | (empty; findings on stderr/exit code)                | JSON findings                              |
| `forge snapshot`       | Snapshot file content / diff                         | JSON                                       |
| `forge coverage`       | Coverage table or report                             | JSON / LCOV / etc. via `--report`          |
| `forge cache`          | (empty) or paths                                     | JSON                                       |
| `forge doc`            | (empty)                                              | n/a                                        |
| `forge generate`       | (empty) or generated path                            | n/a                                        |
| `forge soldeer`        | (empty)                                              | n/a                                        |
| `forge remappings`     | One remapping per line                               | n/a                                        |
| `forge compiler`       | Compiler info                                        | JSON                                       |
| `forge verify-contract`| Verification GUID / URL                              | JSON                                       |

### `anvil`, `chisel`, `script`

| Command       | Text mode stdout                              | `--json` stdout       |
| ------------- | --------------------------------------------- | --------------------- |
| `anvil`       | Banner, accounts, RPC URL on stderr           | n/a                   |
| `chisel`      | REPL output                                   | n/a                   |
| `forge script`| Simulation/broadcast result                   | JSON                  |

## References

- Macros: [`crates/common/src/io/macros.rs`](../../crates/common/src/io/macros.rs)
- Shell wrapper: [`crates/common/src/io/shell.rs`](../../crates/common/src/io/shell.rs)
- Spinner / progress: [`crates/common/src/term.rs`](../../crates/common/src/term.rs)
- Lint configuration: [`clippy.toml`](../../clippy.toml)
