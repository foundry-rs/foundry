# Foundry agent-facing substrate

This directory contains the specification for Foundry's agent-facing surface:
the stable, machine-readable contract that downstream tooling, automation, and
AI agents can rely on across `forge`, `cast`, `anvil`, and `chisel`.

The agent surface is layered on top of the existing CLIs. It is opt-in,
additive, and explicitly versioned. It does not replace the human-facing CLI;
it complements it so that the human interface can evolve without breaking
machine consumers.

## Documents

- [`spec.md`](./spec.md) — the contract (introspection, machine mode,
  envelope, streaming, sessions, versioning, deprecation policy)
- [`exit-codes.md`](./exit-codes.md) — the canonical exit-code table
- [`diagnostics.md`](./diagnostics.md) — diagnostic-code namespacing
- [`schemas/`](./schemas/) — JSON Schema files for every committed `@v1`
  identifier

## Schema identifiers

Every machine-readable artifact has a stable logical identifier of the
form `foundry:<name>@vN` (e.g. `foundry:envelope@v1`,
`foundry:introspect@v1`). The identifier is the contract — agents pin
against it, not against the in-memory shape of any particular release.

Identifiers appear inline as a `schema_id` field on the introspect
document and on stream/session records. Envelope payload schemas are
discovered via the emitting command's `capabilities.result_schema_ref`
in introspection (see [`spec.md`](./spec.md) §8).

The [`schemas/`](./schemas/) directory carries a JSON Schema (Draft
2020-12) for every `@v1` identifier currently shipped. They are the
source of truth for downstream tooling; the in-repo `foundry-test-utils`
crate validates real emitted payloads against them on every CI run.

## Relationship to legacy flags

- `--markdown-help` is **deprecated for agent use**. It targets human
  readers and predates the agent contract; use `--introspect` instead.
- `--json` predates this contract and remains supported for backward
  compatibility with existing scripts. It is **not part of the agent
  surface** — its shape varies by command and is not pinned by any
  `foundry:*@vN` identifier. Use `--machine` to opt in to the contract.

## Retry safety for chain-write commands

Under `--machine`, commands declared with `side_effects: chain_write`
(e.g. `forge script`) submit real transactions. Network errors,
interrupted processes, and lost terminal envelopes can leave
on-chain state ahead of what the envelope reports. **`@v1` does not
guarantee idempotency or safe retries.** The terminal envelope's
`tx_hashes` / `created_contracts` are emitted only on a clean exit;
if the process is killed mid-broadcast, consumers must reconcile
chain state directly (e.g. `cast nonce`, `cast receipt`) before
re-issuing the call.


