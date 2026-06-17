# Foundry agent contract

This document defines the agent-facing contract for Foundry. It applies to
the `forge`, `cast`, `anvil`, and `chisel` binaries.

The contract is layered on top of the existing CLIs and does **not** change
the behavior of legacy `--json` users. New agent semantics are opt-in via
`--machine` and via the discovery flag `--introspect`.

Per-command adoption is signaled in the introspect output itself. A command
reports `command_id_stable=true` once its identifier is pinned in the
binary's registry and `capabilities_declared=true` once its capabilities are
authored. Commands without a registry entry report both as `false`, and
consumers MUST treat the default `capabilities` payload as non-authoritative
(side-effects, project requirement, and the rest are unknown, not safe).

## Forward compatibility within `@v1`

Within a major schema version (e.g. `foundry:introspect@v1`), fields may be
**added** without bumping the schema id. Consumers MUST ignore unknown fields.
Field removals or semantic changes require a new major (`@v2`).

Array order in introspect output follows clap declaration/traversal order and
is **not semantic**. Agents MUST key by `command_id`, argument `name`, etc.,
not by position.

---

## 1. Discovery — `--introspect`

Every binary supports a top-level `--introspect` flag. When passed, the
binary emits a single JSON document on stdout describing itself and exits
with code `0`.

`--introspect` is detected **before** clap argument parsing, so it works
even when the binary's required arguments or subcommands are missing.

The document is identified by `foundry:introspect@v1` and is the same shape
emitted by every Foundry binary.

Top-level shape:

```json
{
  "schema_id": "foundry:introspect@v1",
  "schema_version": 1,
  "binary": {
    "name": "forge",
    "version": "<short>",
    "long_version": "<long>",
    "description": "..."
  },
  "commands": [ /* CommandInfo */ ]
}
```

`CommandInfo` exposes, per command:

- `command_id` — machine identifier (e.g. `forge.build`)
- `command_id_stable` — `true` only when the id is pinned in the binary's
  registry (frozen across CLI renames); `false` means the id is derived from
  the current clap path and may shift until the command is registered
- `path` — clap path components (e.g. `["forge", "build"]`)
- `aliases` — visible aliases for the command name
- `summary`, `description`
- `args[]` — each with `name`, `kind` (`flag`/`option`/`positional`),
  `value_type` (best-effort UI hint, not a normative invocation schema),
  `help`, `long`, `short`, `aliases`, `env`, `default`, `possible_values`,
  `required`, `repeatable`, `conflicts_with`, `help_heading`. The
  `conflicts_with` field is **best-effort and may be empty**: clap's public
  API does not expose conflict relationships from an `Arg`, so populating
  this field requires per-binary annotations. When non-empty, it is
  authoritative; when empty, agents MUST NOT assume the absence of conflicts.
- `subcommands[]` — recursive `CommandInfo`
- `capabilities` — see §3
- `capabilities_declared` — `true` only when the registry authored the
  capabilities for this command; when `false`, every capability field is a
  non-authoritative default and consumers MUST treat side-effects, project
  requirement, etc. as unknown
- `exit_codes[]` — command-specific exit codes (in addition to the global set)

### Example `CommandInfo`

```json
{
  "command_id": "forge.build",
  "command_id_stable": true,
  "path": ["forge", "build"],
  "aliases": ["b"],
  "summary": "Build the project's smart contracts",
  "args": [
    {
      "name": "jobs",
      "kind": "option",
      "value_type": "integer",
      "long": "jobs",
      "short": "j",
      "aliases": [],
      "possible_values": [],
      "required": false,
      "repeatable": false,
      "conflicts_with": [],
      "hidden": false
    }
  ],
  "subcommands": [],
  "capabilities": {
    "output_mode": "envelope",
    "result_schema_ref": "foundry:forge.build@v1",
    "reads_stdin": false,
    "supports_output_path": false,
    "requires_project": true,
    "side_effects": "fs_write",
    "long_running": false,
    "stateful": false
  },
  "capabilities_declared": true,
  "exit_codes": []
}
```

---

## 2. Command identifiers

Every leaf and group command has a `command_id`. The default is the clap path
joined by `.` (e.g. the command at `forge` → `build` has `command_id`
`forge.build`).

For commands promoted to `stable`, the `command_id` is **frozen** in the
in-binary registry so subsequent renames or reorganizations of the human-
facing CLI do not move the machine identifier.

`command_id` values are unique within a binary. CI enforces uniqueness.

---

## 3. Command capabilities

Each `CommandInfo.capabilities` block exposes machine-relevant behavior:

| Field                  | Type                                                                | Meaning                                                                  |
| ---------------------- | ------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| `output_mode`          | `none` \| `legacy_json` \| `envelope` \| `stream` \| `session`      | What the command emits when run in machine mode                          |
| `result_schema_ref`    | string \| null                                                      | Stable schema id for the envelope payload (e.g. `foundry:forge.build@v1`) |
| `event_schema_ref`     | string \| null                                                      | Stable schema id for stream event records                                |
| `session_schema_ref`   | string \| null                                                      | Stable schema id for session-record startup/state                        |
| `reads_stdin`          | bool                                                                | Whether the command can take input via `--input -`                       |
| `supports_output_path` | bool                                                                | Whether the command supports `--output PATH`                             |
| `requires_project`     | bool                                                                | Whether the command needs a Foundry project                              |
| `side_effects`         | `none` \| `fs_write` \| `network` \| `chain_write` \| `spawn_server`| Highest-impact side-effect class (not an exhaustive set)                 |
| `long_running`         | bool                                                                | Whether the command can stream output for an extended period             |
| `stateful`             | bool                                                                | Whether the command opens a session that persists beyond a single call   |

These fields let agents decide how to consume each command without parsing
prose help text.

---

## 4. Output modes

Output mode is reported per command in `capabilities.output_mode`.

- **`none`** — no machine-mode contract yet; output is human-only.
- **`legacy_json`** — pre-existing `--json` shape predating this contract.
  Not part of the agent contract; preserved for backward compatibility only.
- **`envelope`** — single terminal `JsonEnvelope<T>` on stdout; payload `T`
  is described by `result_schema_ref`.
- **`stream`** — newline-delimited JSON event records on stdout, optionally
  ending with a final terminal record. Event shape is described by
  `event_schema_ref`. Consumers must tolerate a missing final record on
  `SIGINT` / `SIGKILL`.
- **`session`** — long-running session command (e.g. `anvil`). Emits a
  `session_start` record on startup; subsequent records may follow. Not a
  terminal envelope.

`--json` continues to mean what it means today (legacy single-object output
for read commands; pre-existing JSON shapes elsewhere). The new agent
contract activates under `--machine`.

---

## 5. Envelope

The terminal-result envelope is identified by `foundry:envelope@v1` and
implemented by [`JsonEnvelope<T>`](../../crates/cli/src/json.rs):

```json
{
  "schema_version": 1,
  "success": true,
  "data": { /* per-command payload */ },
  "errors": [],
  "warnings": []
}
```

Diagnostics inside `errors[]` and `warnings[]` carry a stable `code`
(`JsonMessage.code`) drawn from the diagnostic registry — see
[`diagnostics.md`](./diagnostics.md).

---

## 6. Streaming events

Stream commands write one JSON object per line on stdout. Each record carries:

- `schema_id` — the event-schema id (e.g. `foundry:forge.test.event@v1`)
- `command_id` — the emitting command (e.g. `forge.test`)
- `kind` — record kind (per-event-schema enum)
- `ts` — RFC 3339 timestamp, UTC, millisecond precision (e.g.
  `2026-05-28T17:15:42.123Z`). Fixed-width substring; safe for regex pinning
  and log search.
- additional kind-specific fields

A stream may end with a single terminal `JsonEnvelope` record on the same
stream. Consumers MUST tolerate streams that end without it (e.g. on signal
termination).

### Per-command event ordering

For each per-command event schema (e.g. `foundry:forge.test.event@v1`),
each grouping unit (suite for `forge.test`, simulation phase for
`forge.script`, etc.) emits records in this order:

1. zero or more child records (e.g. `test_result`)
2. zero or more non-terminal annotations on that group (e.g. `warning`)
3. exactly one terminator record for the group (e.g. `suite_finished`)

After a group's terminator, no more records targeting that group may be
emitted. Groups themselves are not ordered against each other; agents
should key on the group identifier in the payload (e.g. `suite` for
`forge.test`) when correlating per-group records.

### Warning duality

When a command surfaces a warning that is also relevant to the terminal
outcome, the warning is emitted **twice**:

1. as a per-suite `warning` stream event with `kind: "warning"` and the
   per-event `code` (e.g. `test.warning`) — for real-time visibility
2. as an entry in the terminal envelope's `warnings[]` — for the
   end-of-run summary

Both surfaces carry the same `code` and `message` and identify the suite
via the same `suite` key — `suite` as a top-level field on the stream
event, and `details.suite` on the terminal envelope warning. Agents
consuming both surfaces should de-duplicate by `(suite, message)`. The
terminal envelope's `warnings[]` is the authoritative aggregated set.

### Failure-envelope conventions

A command's `result_schema_ref` describes the envelope `data` payload
when present. Failure envelopes may set `data: null` and surface
structured failure context inside `errors[].details`. Agents that need
to detect failure key on the combination of exit code, `success: false`,
and `errors[].code`; per-command spec sections document where additional
context lives on failure (for `forge.test`, the summary appears under
`errors[0].details` and is the same shape as `data` on success).

### Concrete shapes (informative, `@v1`)

The wire contract is the schema id and the field set documented per
command in [`crates/forge/src/introspect.rs`](../../crates/forge/src/introspect.rs)
and the equivalent registry files. The current `forge.test` event payloads
under `foundry:forge.test.event@v1`:

- `kind: "test_result"` — `{ suite, name, status, reason?, duration_ms }`
  with `status ∈ { "passed", "failed", "skipped" }`.
- `kind: "warning"` — `{ suite, code, message }`.
- `kind: "suite_finished"` — `{ suite, passed, failed, skipped, duration_ms }`.

`suite` is the full suite identifier (e.g. `test/Counter.t.sol:CounterTest`).
A `suite_finished` record terminates the group for that suite; no further
records targeting that suite are emitted. Warning-only suites still emit a
`suite_finished` (with zero counts) so the group lifecycle is honest.

The terminal envelope payload under `foundry:forge.test@v1`:
`{ suites, passed, failed, skipped, duration_ms }`. When `--allow-failure`
tolerated failures, `success: true` and `data.failed` may be non-zero — see
the `Success` exit-code description on the per-command introspection. On
test failure (`success: false`, `errors[0].code: test.failed`), the same
payload appears under `errors[0].details`.

---

## 7. Sessions

`anvil` and other long-running, stateful commands emit a `session_start`
record on startup carrying:

- `rpc_url`
- `chain_id`
- `accounts`
- `fork_url` (optional)
- `block`

This is **not** a terminal envelope. Subsequent session records may follow.
The session-record schema is `foundry:anvil.session@v1`.

### Root/default command surface (anvil, chisel)

Binaries whose root command is invocable without a subcommand (e.g. `anvil`
starts a JSON-RPC server, `chisel` opens a REPL) expose that default
invocation as a synthetic top-level `CommandInfo` with `path = [<binary>]`
(e.g. `["anvil"]`, `["chisel"]`). Its `args[]` lists the root-only,
non-global options accepted by the default invocation (`--port`,
`--fork-url`, ...). Truly clap-global options remain reported once on
`binary.global_args`; they are not duplicated onto the synthetic root
command.

Until each binary pins a registry entry at the empty path, the synthetic
root command's `command_id` is derived (e.g. `"anvil"`, `"chisel"`) and
ships with `command_id_stable=false` and `capabilities_declared=false`.
Pinning stable ids such as `anvil.start` / `chisel.repl` and emitting the
`session_start` record described above are tracked as follow-up work.

---

## 8. Versioning

Versions are tracked on **four independent axes**:

1. **Envelope** — `foundry:envelope@vN`
2. **Per-command result** — `foundry:<command_id>@vN`
3. **Per-stream event** — `foundry:<command_id>.event@vN`
4. **Per-session record** — `foundry:<command_id>.session@vN`

A bump on one axis does not imply bumps on others.

Where the schema id appears:

- **Introspect document** — top-level `schema_id` field
- **Stream event records** — top-level `schema_id` per record
- **Session records** — top-level `schema_id` on the `session_start` record
- **Envelope payloads** — the envelope itself does not carry a `schema_id`;
  the payload's schema is discovered via the emitting command's
  `capabilities.result_schema_ref` in introspection. This avoids forcing
  every successful envelope to carry version metadata that is already
  pinned by the `command_id` + introspection contract.
- **Global (non-command) machine-mode payloads** — a small fixed set of
  envelopes is emitted by the CLI runtime itself, before any command is
  resolved (see §10 below). Their payload schemas are listed by name in
  this document rather than discovered via introspection, because no
  `command_id` exists at the point of emission.

---

## 9. Deprecation policy

Breaking changes to a schema's wire format require bumping the affected
schema id (`foundry:envelope@v1` → `foundry:envelope@v2`). Both versions
SHOULD be emitted in parallel for at least one minor release before the
older identifier is removed, giving consumers time to migrate.

Removing a `command_id` follows the same rule: the command should remain
functional through one minor release after the announcement, after which
introspection no longer lists it.

---

## 10. Machine mode (`--machine`)

`--machine` is the stable agent-contract selector. The selector itself is
shipped by the CLI runtime; per-command behavior is adopted incrementally.

The runtime guarantees today, regardless of which command is invoked:

- color is disabled (`ColorChoice::Never`)
- parse and usage failures are wrapped in an error envelope
  (`cli.usage.invalid`, exit `2`)
- `--help` / `--version` are wrapped in a success envelope (exit `0`)

The per-command guarantees a command opts into, once adopted, are:

- emits its declared `output_mode` only on stdout
- suppresses progress bars and interactive prompts
- maps process-exit failures to the canonical `ExitCode` enum

Agents may also rely on `--introspect` for discovery and on the existing
`--json` flag for legacy machine-readable output.

### Global machine-mode payload schemas

Two payloads are emitted by the CLI runtime itself (before any command is
resolved) and therefore cannot be discovered via
`capabilities.result_schema_ref` on a command. Their wire shapes are
fixed and listed here as part of the contract:

- **`foundry:cli.help@v1`** — emitted as the `data` of a success envelope
  when `--help` / `-h` is requested under `--machine`. Exit code: `0`.

  ```json
  { "help": "<clap-rendered help text for the actual command context>" }
  ```

  The `help` string is clap's own rendered help for the precise
  subcommand requested (e.g. `cast call --machine --help` carries
  `cast call` help, not `cast` root help).

- **`foundry:cli.version@v1`** — emitted as the `data` of a success
  envelope when `--version` / `-V` is requested under `--machine`.
  Exit code: `0`.

  ```json
  { "version": "<clap-rendered version text>" }
  ```

Usage failures (missing subcommand, missing required arg, conflict,
unknown flag, including clap's `DisplayHelpOnMissingArgumentOrSubcommand`
case which renders help on missing input) are emitted as an error
envelope with diagnostic code `cli.usage.invalid` and exit code `2`.

---

## 11. I/O conventions

- `--input -` reads from stdin; `--input PATH` reads from a file
- `--output -` writes to stdout (default); `--output PATH` writes to a file
- Compact JSON (no pretty-printing) on every machine-mode output

---

## 12. Out of scope

- Cancellation tokens / idempotency keys
- Output paging or chunked file protocols
- Bidirectional RPC sessions for `anvil`/`chisel`
- A general agent transport beyond CLI stdout/stderr

These remain for future versions if real consumers need them.
