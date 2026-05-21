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
- `ts` — RFC 3339 timestamp
- additional kind-specific fields

A stream may end with a single terminal `JsonEnvelope` record on the same
stream. Consumers MUST tolerate streams that end without it (e.g. on signal
termination).

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

`--machine` is the stable agent-contract selector. When set, a command:

- emits its declared `output_mode` only
- never writes color, progress bars, or interactive prompts to stdout
- wraps parse and usage failures in an error envelope, and wraps help/version
  output in a success envelope (instead of plain text on stderr/stdout)
- maps process-exit failures to the canonical `ExitCode` enum

Agents may also rely on `--introspect` for discovery and on the existing
`--json` flag for legacy machine-readable output.

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
