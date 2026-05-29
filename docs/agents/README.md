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

## Schema identifiers

Every machine-readable artifact has a stable logical identifier of the
form `foundry:<name>@vN` (e.g. `foundry:envelope@v1`,
`foundry:introspect@v1`). The identifier is the contract — agents pin
against it, not against the in-memory shape of any particular release.

Identifiers appear inline as a `schema_id` field on the introspect
document and on stream/session records. Envelope payload schemas are
discovered via the emitting command's `capabilities.result_schema_ref`
in introspection (see [`spec.md`](./spec.md) §8).

Concrete JSON Schemas describing each identifier are introduced alongside
the commands that adopt them.


