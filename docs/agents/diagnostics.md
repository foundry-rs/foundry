# Diagnostic codes

Every `JsonMessage` carried in an envelope's `errors[]` or `warnings[]`
includes a stable, machine-readable `code`. Codes are strings drawn from a
namespaced registry. They are part of the agent contract — agents may switch
on them.

## Format

A diagnostic code is a dot-separated, lowercase, ASCII-only string of the
form:

```
<domain>.<subdomain>[.<subdomain>...]
```

Examples:

- `config.invalid`
- `config.missing_field`
- `compiler.solc.error`
- `compiler.vyper.error`
- `network.rpc.timeout`
- `network.rpc.unauthorized`
- `wallet.key.missing`
- `wallet.signature.rejected`
- `test.failed`
- `test.setup_failed`
- `script.broadcast_failed`

## Reserved domains

| Domain     | Owner crate / area              | Examples                                  |
| ---------- | ------------------------------- | ----------------------------------------- |
| `config`   | `foundry-config`                | `config.invalid`, `config.missing_field`  |
| `compiler` | `foundry-compilers`, `forge`    | `compiler.solc.error`                     |
| `network`  | RPC / HTTP layers               | `network.rpc.timeout`                     |
| `wallet`   | `foundry-wallets`               | `wallet.key.missing`                      |
| `test`     | `forge test`                    | `test.failed`, `test.setup_failed`        |
| `script`   | `forge script`                  | `script.broadcast_failed`                 |
| `cast`     | `cast`                          | `cast.tx.not_found`                       |
| `anvil`    | `anvil`                         | `anvil.fork.unreachable`                  |
| `chisel`   | `chisel`                        | `chisel.session.invalid`                  |
| `cli`      | argument parsing / global flags | `cli.usage.invalid`                       |

New domains require a PR that updates this table.

## Implementation shape

Codes are **not** modeled as one monolithic repo-wide enum. Two patterns
are allowed:

1. **`DiagnosticCode` newtype over `String`** with namespaced `pub const`
   declarations colocated with each owning crate, or
2. **per-domain enums** (`ConfigDiagnostic`, `NetworkDiagnostic`, …) that
   serialize to namespaced strings.

A repo-wide test asserts every emitted code matches the format above and
appears in this document.

## Versioning

Codes are part of the schema they appear in. Removing or renaming a code
requires bumping the affected schema id (envelope `@vN` for global codes;
per-command `@vN` for command-local codes), following the deprecation
policy in [`spec.md`](./spec.md) §9.
