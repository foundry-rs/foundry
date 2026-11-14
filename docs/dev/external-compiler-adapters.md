# External compiler adapters

Foundry can compile compiler-native EVM projects through explicitly configured executable
adapters. The adapter owns project discovery, dependency resolution, and compiler invocation.
Foundry owns process transport, cache validation, artifact paths, and integration with Forge.
This implements the protocol tracked by [OSS-821][issue].

This keeps language-specific parsers and toolchains outside Foundry while making their EVM
artifacts available to the existing build, test, script, inspection, binding, selector, and create
flows. Solidity and Vyper continue to use their built-in compiler paths.

## Configuration

Configure each adapter under the active profile:

```toml
[[profile.default.external_compilers]]
id = "contracts"
command = "/opt/toolchains/fe-foundry-adapter"
args = []
roots = ["contracts"]

[profile.default.external_compilers.settings]
compiler = "/opt/toolchains/fe"
optimization = "s"
```

`id` is a unique namespace for the adapter's artifacts and cache entries. It may contain ASCII
letters, digits, `.`, `-`, and `_`, but may not be `.` or `..`. `command` is an absolute path or a
path relative to the Foundry project root. Foundry executes it directly without a shell or PATH
lookup. `roots` contains one or more compiler-native project roots relative to the Foundry project.
`args` is an optional argument array, and `settings` is an optional JSON-compatible TOML table
passed through without interpretation.

Configuring an adapter authorizes that executable to run with the user's privileges. Foundry clears
the child environment and starts it in the Foundry project root, but this process boundary is not
an operating-system sandbox. Adapters that need environment variables or network access must
arrange those requirements themselves through their executable or explicit settings.

## Transport and lifecycle

Protocol version 1.0 uses newline-delimited JSON on stdin and stdout. One adapter process serves one
Forge compilation. Requests are sequential, use increasing integer IDs, and receive a response with
the same ID and exactly one of `result` or `error`. Stdout is reserved for protocol messages. The
adapter may use stderr for failure context; structured compiler diagnostics belong in the compile
result. Foundry rejects individual protocol lines larger than 16 MiB.

Foundry sends three operations in order.

### `initialize`

```json
{"id":1,"method":"initialize","params":{"protocols":["1.0"],"host":{"name":"forge","version":"1.8.4"},"target":"evm"}}
{"id":1,"result":{"protocol":"1.0"}}
```

The adapter must select exactly `1.0`. Protocol and compiler versions are independent.

### `discover`

```json
{"id":2,"method":"discover","params":{"roots":["/project/contracts"],"settings":{"optimization":"s"},"selected_paths":[]}}
{"id":2,"result":{"units":[{"id":"app","compiler":{"name":"fe","version":"26.3.0"},"inputs":["contracts/fe.toml","contracts/src/lib.fe"],"capabilities":["build/1"],"effectiveSettings":{"optimization":"s"},"cacheable":true}]}}
```

The adapter returns complete build units. A unit is the smallest compiler-native project that can
be built independently. Its `inputs` must contain every file whose bytes determine compilation,
including manifests, lockfiles, local dependency sources, and compiler resources when needed.
Paths may be absolute or relative to the Foundry root and must resolve to files. The compiler
version must be valid SemVer.

Every unit must advertise `build/1`. Advertising `forge-tests/1` additionally makes deployable
artifacts from that unit eligible for Forge's test-contract discovery. Build-only artifacts remain
available as known contracts and deployment dependencies during `forge test`; ABI function names
alone do not opt them into execution as test suites.

`selected_paths` contains explicit paths selected by the calling Forge command. The adapter decides
which units own those paths and must still return each selected unit's complete input closure. An
empty array requests complete discovery, so the adapter must return every active unit.

### `compile`

Foundry sends `compile` only on a cache miss or when caching is disabled:

```json
{"id":3,"method":"compile","params":{"unit":"app","fingerprint":"<sha256>"}}
{"id":3,"result":{"diagnostics":[],"artifacts":[{"source":"contracts/src/lib.fe","name":"Counter","contract":{"abi":[],"evm":{"bytecode":{"object":"0x60006000f3"},"deployedBytecode":{"object":"0x00"}}},"metadata":{"language":"Fe"}}]}}
```

The result may contain `error`, `warning`, or `info` diagnostics. Error diagnostics fail the Forge
command. Each artifact supports these fields:

| Field | Required | Meaning |
| --- | --- | --- |
| `source` | yes | Project-relative path to an existing source file. |
| `name` | yes | Contract name. |
| `contract` | yes | Existing compiler `Contract` JSON: `abi` and `evm` outputs. |
| `metadata` | no | Adapter-owned JSON metadata, serialized into Foundry's `rawMetadata` field. |
| `sourceId` | no | Source ID used by the supplied source maps. |

`contract.evm.bytecode` and `contract.evm.deployedBytecode` use the existing compiler bytecode
objects (`object`, `sourceMap`, `linkReferences`, and runtime `immutableReferences`). Bytecode must
be fully linked: unresolved objects and nonempty link references are rejected. Runtime bytecode
requires creation bytecode. Foundry derives method identifiers from the ABI and uses its existing
artifact converter to produce ABI/bytecode artifacts. Debugging and additional compiler outputs
remain outside this protocol's initial integration.

Source paths must be relative and may not contain `.` or `..` components. Unit IDs use the same
restricted character set as adapter IDs. Contract names permit only ASCII letters, digits, and
underscores so binding generation preserves their identity. Source identities are canonicalized
to match path-qualified Forge commands; virtual source files are not supported. Foundry rejects duplicate
`(source, contract)` identities across adapters and conflicts with built-in compiler artifacts.

After the last response Foundry closes stdin and requires the adapter to exit successfully. A
protocol error, compiler error diagnostic, malformed artifact, or nonzero exit fails the Forge
command; Foundry does not fall back to another compiler.

## Cache and artifacts

Discovery runs on every build, including cache hits. For each cacheable unit Foundry hashes the
protocol version, adapter executable path, bytes and arguments, requested and effective settings,
compiler identity, unit descriptor, and the path and contents of every discovered input.
Compilation is independent of the Forge command consuming the result, so `build`, `test`, and
inspection can reuse the same unit. Build variants must be expressed in adapter settings and the
unit's effective settings, rather than inferred from a command name.
The SHA-256 fingerprint indexes the unit result under
`cache/external-compilers/<adapter>/<unit>.json`. `cacheable = false`, `cache = false`, and
`--force` bypass reuse.

Normalized artifacts are written under:

```text
out/.external/<adapter>/<unit>/<source>/<contract>.json
```

They are merged into `ProjectCompileOutput`, so Forge's artifact lookup, linking, tracing, scripts,
bindings, selectors, inspection, and contract creation can consume them. Foundry removes artifact
directories for adapters and units that disappear from complete discovery, and replaces the active
unit directory so contracts no longer emitted by a unit are retired. `forge clean` removes these
host-owned artifacts and cache entries with the normal `out` and `cache` directories.

The adapter cache is unit-scoped and independent of the Solidity/Vyper compiler cache. Read-only
compilations (`forge inspect`, `forge test --list`, and selector queries) may reuse it but do not
publish artifacts or update or retire cache entries. `--force` (or `force = true`) retains Forge's
existing cleanup semantics: it deletes prior artifacts and caches before compilation, including
for read-only commands, which do not republish external outputs. Use `forge build` to restore them.
Declaring a unit cacheable promises that discovery lists its complete build-affecting input closure. Ambient inputs such as time,
randomness, undeclared environment, mutable dependency caches, or unreported compiler resources require `cacheable = false`.

## Forge integration and limits

Adapter-aware compilation is enabled for `forge build`, `forge test`, scripts, `forge inspect`,
`forge bind`, `forge selectors`, `forge create`, and `forge coverage`. Adapter-only projects bypass
the built-in "Nothing to compile" exit. Mixed projects compile the external units first, then the
built-in sources, and merge their normalized artifacts only after both compilers succeed.

Commands that select one external contract before compilation, such as `forge inspect` and
`forge create`, require a path-qualified identifier such as `contracts/src/lib.fe:Counter` because
Foundry cannot infer an adapter-owned source path from a contract name before discovery.

`forge coverage` can execute external contracts while reporting coverage for supported Solidity
sources. External sources are excluded from source analysis and coverage reports. Source-level
debugging and verification reproduction for external artifacts remain outside the initial integration.
Build and test execution still work without those optional workflows. Cross-compiler source imports
and generated interface dependency scheduling are not supported; use ABI interfaces or artifact
deployment at the language boundary.

The host does not install adapters, compilers, or native dependencies, and does not interpret
language manifests. Adapter distribution, toolchain selection, dependency setup, protocol
conformance, and native compiler diagnostics remain the adapter maintainer's responsibility.

[issue]: https://linear.app/tempoxyz/issue/OSS-821
