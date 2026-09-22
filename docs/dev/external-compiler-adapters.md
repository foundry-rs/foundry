# External compiler adapters (proposal)

Status: proposed, not implemented. Tracks [OSS-821][issue]. Configuration and wire examples below
describe a candidate protocol, not options available in released Foundry.

## Decision and scope

Add an explicitly configured executable adapter that owns a **complete compiler-native project
build**. Foundry supplies project roots, requested workflows, and common EVM requirements; the
adapter discovers manifests, sources, and dependencies, invokes its installed compiler, and returns
normalized artifacts. A standalone file can be a native project when the adapter supports it.
Foundry must not split a native project into per-file invocations or parse its dependency manifest.

Version 1 targets EVM bytecode and Solidity-compatible ABI. Keep Solidity and Vyper on their existing
paths. Exclude in-process plugins, automatic compiler installation or updates, non-EVM targets, and
universal workflow parity. In particular, the RISC-V output discussed in [#10021][revive] is outside
this proposal; ABI compatibility alone does not make executable code EVM-compatible. The alternate
compiler request in [#4418][alternate] motivates the executable boundary.

## Configuration and trust

An illustrative configuration for an independently distributed Fe adapter:

```toml
[[profile.default.compiler_adapters]]
id = "contracts"
command = "/opt/toolchains/fe-foundry-adapter"
args = []
roots = ["contracts"]

[profile.default.compiler_adapters.settings]
compiler = "/opt/toolchains/fe"
optimization = "s"
```

`id` is a unique, user-chosen namespace, not a language enum. Roots resolve relative to the Foundry
project. `command` is an absolute path or an explicit project-relative path; launch it with an argv
array and no shell. No PATH search, extension-triggered execution, or adapter registry lookup occurs.
`settings` is an opaque JSON-compatible TOML table interpreted and validated by the adapter. The
example's `compiler` and `optimization` keys have no Fe-specific meaning in Foundry. Normal profile
resolution applies; duplicate IDs and overlapping ownership of the same native build are errors.
Native dependency sharing does not itself constitute an ownership conflict.

Configuring an adapter authorizes local executable code, including discovery. This is a process
boundary, **not an OS sandbox**: the child has the user's privileges. Start it at the Foundry project
root with an explicit minimal environment; any additional inherited variable names must be
configured and included in cache identity. The adapter must declare its build-affecting environment
and toolchain inputs. Foundry does not send RPC credentials or verification tokens. Adapters own
compiler selection and dependency resolution, but v1 requires dependencies to be already available
locally; missing dependencies fail with native setup instructions. No implicit network fetches,
manifest/lockfile rewrites, installation, or updates are part of discovery or compilation. These
are adapter obligations; hostile executables require external sandboxing.

The host supplies a private scratch/output directory. Validate output paths, canonical containment
(including symlinks), IDs, sizes, bytecode, references, and source spans before importing anything.
Native input paths may legitimately lie outside the project for local dependencies; keep them
distinct from output destinations. Adapters never choose destinations under Foundry's `out/` or
`cache/`. Apply configurable process and output limits, drain stderr while reading stdout, and
terminate the child process group on timeout or cancellation. Choose numeric defaults during the
prototype from representative builds, before enabling the feature by default.

## Protocol and lifecycle

Use UTF-8 newline-delimited JSON over stdin/stdout, sequential request/response messages with an
integer request ID, and stderr for human progress. One process serves one host build session, with
no persistent daemon. Stdout contains protocol messages only. Each response echoes the ID and has
exactly one of `result` or `error`; a normal compiler failure is a result with error diagnostics.
After the final response, the host closes stdin and requires a successful process exit before
publishing results. Adapters stop their compiler children before exiting.

```json
{"id":1,"method":"initialize","params":{"protocols":["1.0"],"host":{"name":"forge"},"target":"evm"}}
{"id":1,"result":{"protocol":"1.0","adapter":{"name":"fe-foundry-adapter","version":"0.1.0"},"extensions":[]}}
```

The initial envelope is stable across versions. The adapter selects one exact host-offered protocol
or returns `unsupported_protocol`. Breaking envelope/artifact semantics require a new major;
additive optional fields and named capability schemas use minor versions. Unknown optional fields
are ignored, unknown required features are rejected, and unknown capabilities are never enabled.
Protocol version and compiler version are independent.

| Operation | Request | Result and host action |
| --- | --- | --- |
| `initialize` | Supported protocol versions and target family. | Negotiate before interpreting any project output. |
| `discover` | Configured roots/settings, required EVM revision, requested workflows/outputs. | Return native build units, full input closure, effective settings, compiler/toolchain identities, and per-unit capabilities. Always run before a host cache lookup. |
| `compile` | Unit ID, discovery fingerprint, requirements, host-owned scratch directory. | Build the entire unit; return diagnostics and a normalized artifact bundle with actual input provenance. Reject stale discovery. |

Discovery may return several units, for example a workspace's independently buildable members. The
adapter defines their boundaries and transitive dependencies; the host treats each unit as opaque
and atomic. Report stable unit IDs, member/entrypoint ownership, source and manifest paths, local
dependency roots, resolved dependency identities, toolchain binaries/resources, and effective
settings after native defaults. Settings the adapter cannot honor, including the EVM revision,
produce an error rather than a silently different build. A configured root with no recognized
project is an error; an adapter-only project must bypass today's "Nothing to compile" shortcut.

File filters select owning units; they never truncate their input closure. Build each selected unit
once, then filter its returned artifacts. Discovery runs before built-in parsing; adapter-owned
entrypoints are excluded from built-in builds, and conflicting ownership is rejected. Mixed
Solidity/adapter projects compile independently and merge at the artifact boundary. Cross-language
calls use ABI interfaces or artifact deployment; cross-compiler source imports and
generated-interface dependency scheduling are deferred.

## Artifacts and optional workflows

The mandatory bundle contains a stable `(adapter ID, unit ID, source unit, contract name)` identity,
compiler identity, effective settings, EVM revision, a source table, diagnostics, and contracts.
Each contract has a standard JSON ABI and separate creation/runtime bytecode objects; distinguish
an absent deployable bytecode (interface) from valid empty bytecode. Source IDs are local to a build
unit and refer to exact source content and hashes, including generated sources. Keep logical source
names separate from filesystem paths. Namespace source IDs and source-map lookups by build identity
when merging; preserve the original bundle.
Never overwrite artifacts on collisions or substitute a fictitious solc version for another compiler.

Diagnostics include severity, a namespaced string code, message, and optional primary/related spans
using source IDs and UTF-8 byte offsets. Preserve native metadata exactly as opaque text/bytes with
a media type and schema identifier; do not parse and reserialize away bytecode-relevant content.
Explicit absence is valid for metadata and source maps. Optional creation/runtime source maps use
a negotiated schema (initially Solidity's instruction-index source-map encoding), reference that
bundle's sources, and are checked against the corresponding bytecode. Persist the normalized
bundle, native metadata, and build provenance on both fresh and cached paths.

Capabilities identify versioned data/behavior contracts, not a claim that all Foundry commands work.
The usable feature set is the intersection of host support, adapter support, and outputs actually
provided by this unit. Validate requested capabilities before compilation and actual outputs before
execution. Missing capabilities produce a specific unsupported-workflow error.

Requirements attach to consumption roles. Deployment dependencies need build and, when applicable,
linking support; artifacts considered as external test suites need explicit `forge-tests/1`
eligibility. ABI naming alone never opts an external artifact into test execution. This allows a
Solidity test to deploy an adapter artifact without requiring that artifact to be a native test suite.

| Workflow | Contract and initial behavior |
| --- | --- |
| Build / ABI consumption | Required. ABI, bytecode, diagnostics, provenance; contract name selection remains unambiguous. |
| Linking | Fully linked bytecode needs no optional support. `link-references/1` supplies validated creation/runtime offsets, lengths, and library artifact IDs for the existing host linker. Reject unresolved references without support. Native linking can instead consume explicitly supplied addresses as fingerprinted settings. |
| Forge tests | `forge-tests/1` marks eligible artifacts following Forge ABI naming/setup conventions and deployment rules. Includes ordinary ABI-driven fuzzing where supported; native compiler test harnesses are not implicitly Forge tests. Solidity tests may deploy adapter artifacts without this capability. |
| Debugging | Opcode tracing remains possible. `source-maps/1` enables source stepping only after generic host integration; locals/scopes require a separate schema. Do not send unknown languages through Solar. |
| Coverage | Unsupported in initial v1. Source maps alone cannot replace Solidity AST-based statement/branch analysis. Reserve a future coverage-items schema and host consumer. Explicit coverage requests fail for affected units. |
| Verification | Optional `reproduction-bundle/1` exports exact sources, compiler identity, settings, libraries, and native metadata. Network verification additionally requires a compatible host provider; the bundle alone does not enable Etherscan/Sourcify. No arbitrary adapter-selected HTTP requests. |

Formatting, linting, mutation, Solidity AST bindings, and language-specific inline configuration are
not implied by successful compilation. Commands must reject unsupported selected inputs. Ordinary
build/test commands may operate without source debugging, coverage, or verification; reports must
not silently claim those features or complete coverage of omitted languages.

## Cache and failure semantics

Cache a whole native unit, independently of Solidity's per-source compiler cache. Discovery is
mandatory even on a hit, so new/deleted source files, changed workspace membership, and new manifest
dependencies are visible. The host computes a versioned content hash over:

- Protocol/artifact schema versions and the host normalization version.
- Adapter executable content, resolved path/argv, and adapter-reported implementation resources
  (including interpreter/package files for script adapters).
- Every compiler/backend binary and resource identity, not just its display version.
- Unit identity/root, complete discovered input path/content set, dependency resolution, manifests,
  lockfiles when present, and generated input provenance.
- Opaque requested settings, resolved effective settings, EVM revision, requested output/capability
  set, library addresses, and the effective build-affecting environment.

Use a specified canonical serialization (including deterministic map ordering) and SHA-256, not
mtime or process-local hashes. Hash local bytes in the host; require content-addressed identities
for non-file resources. A toolchain or dependency closure that cannot be fingerprinted completely
must report `cacheable: false`; the host then always invokes compilation. Cacheability asserts that
the result is determined by the complete declared build context, including ambient and platform
inputs and generated-input provenance. Time, randomness, directory enumeration, mutable native
caches, or any other undeclared input also require `cacheable: false`. Adapter private caches must
not reintroduce reuse based on incomplete inputs. The host cannot detect a dishonest declaration.
Never persist raw secret environment values in build-info.

Compilation must use the discovered snapshot or fail if it changes. The host checks the reported
actual input set and revalidates discovery/content after compilation before publishing; a changed
input closure invalidates the result. A trusted adapter is responsible for snapshot consistency
during its native build; post-build hashing alone is not a hermetic-build guarantee. Cache hits also
require intact artifact bundles and sufficient outputs. Initially require exact output/capability
set equality, leaving superset reuse as a later optimization.

Missing executable, unsupported protocol/target/settings/workflow, invalid messages, oversized
output, malformed artifacts, compiler errors, nonzero exit, or cancellation fail the command with
adapter/unit context and bounded stderr. A process exiting before its pending response is complete
is a failure. Never fall back to another compiler or stale success. In adapter-aware builds, the
coordinator must prevent the built-in compiler from publishing directly: stage both built-in and
adapter outputs, then publish artifacts and cache entries only after every selected pipeline
succeeds. This requires an additive core entry point that separates compilation from persistence.
Reconcile active unit indexes against complete discovery and resolved adapter configuration; retire
disappeared units, and remove contracts a remaining unit no longer emits. Artifact filters affect the
returned view, not the complete persisted unit bundle, and unselected existing units are not treated
as deleted. Existing artifacts may remain on disk after failure but cannot be returned as a
successful result. `--force` bypasses reuse; `forge clean` removes host-owned adapter artifacts/cache,
not native project files.

## Ownership and integration

The current [`Compiler`/`ParsedSource`/`Language` traits][compiler-traits] assume host-side source
resolution and static file extensions. Adding `Fe` variants would spread language knowledge through
Foundry, as illustrated by [foundry-core#206][core-fe] and [foundry#16968][foundry-fe]. Put a generic
native-project coordinator beside that pipeline rather than forcing foreign manifests through it.

| Owner | Responsibility |
| --- | --- |
| `foundry-core` / `foundry-compilers` | Versioned protocol DTOs and conformance fixtures, process transport, native-unit coordinator/cache, normalized bundles and artifact projection. No Fe parser, dependency, settings type, or compiler variant. |
| Foundry configuration and [`common::compile`](../../crates/common/src/compile.rs) | Generic adapter configuration and host policy, request construction, coordination with built-in compilation, diagnostics and merged artifact lookup. |
| Forge, linking, traces, verification | Consume normalized artifacts; explicitly gate each optional workflow. Keep Solidity analysis on Solidity sources. |
| Adapter maintainers | Native discovery/dependency resolution, compiler invocation and fingerprints, normalization, capability correctness, distribution and version compatibility. |

Prefer additive APIs: preserve existing `Config::project()` and `Project<MultiCompiler>` callers,
introduce the coordinating build entry point for adapter-aware commands, and explicitly reject
configured adapters in unmigrated command paths. This is a real cross-crate change: current
`ProjectCompileOutput`, artifact/source identities, runner analysis, and verification contexts have
built-in compiler assumptions. Audit those consumers before promising transparent compatibility;
do not shoehorn external output into fake Solidity sources or discard provenance during merging.

## Fe walkthrough and validation plan

Fe's documented [ingots][fe-projects] contain `fe.toml` and `src/lib.fe`; [workspaces][fe-workspaces]
add member discovery and shared dependencies. The out-of-tree adapter interprets these and any
standalone-file mode supported by its pinned Fe executable. It owns all Fe version-specific flags
and rejects dependency/target forms it cannot support. Foundry only sees native units and inputs.

For `contracts/fe.toml` depending on a local `lib/step` ingot, discovery returns both manifests and
all relevant sources, the installed Fe toolchain identity, and resolved optimizer/EVM settings.
Compilation invokes the native project build once and translates ABI and both bytecodes into the
bundle. Fe's documented [metadata emission][fe-metadata] can populate the opaque metadata field;
it does not establish compatibility with Foundry's verification providers. Editing `lib/step`,
adding a source/member, changing a manifest, replacing the Fe executable at the same path, or
changing optimization produces a different cache key. All Fe-specific logic lives in the adapter.

This is a design walkthrough, not a working integration or proof of workflow support. Validate it
in three implementation steps:

1. Specify protocol schemas and build a generic fixture adapter. Test incompatible versions,
   malformed output, cancellation, output path escapes, artifact collisions, stale discovery,
   fresh/cache parity, removed outputs and units, filtered builds, uncacheable ambient inputs, and
   input/toolchain/settings invalidation.
2. Implement the coordinator and generic configuration in core/Foundry with configuration tests and
   the corresponding Foundry Book update. Exercise adapter-only and mixed Solidity builds through
   the actual Forge executable, including cache hits, adapter failure after successful built-in
   compilation, Solidity deployment of adapter artifacts, and explicit external test eligibility.
3. Build the Fe adapter in a separate repository against a pinned Fe release. Compile an ingot,
   workspace, and local dependency; run Solidity tests deploying its artifacts, then opt into native
   Forge-ABI test artifacts after testing setup, pass/fail, fuzzing, and cheatcode behavior. Explicitly
   test unsupported coverage/debug/verification requests. Confirm neither Foundry repository gains
   Fe-specific dependencies, configuration fields, file-extension lists, or compiler variants.

Before protocol stabilization, settle the exact artifact projection API and schema, environment
allowlist, and measured resource-limit defaults in that prototype. The default recommendation is to
ship build/artifact interoperability first, then independently qualify optional workflows.

[issue]: https://linear.app/tempoxyz/issue/OSS-821
[alternate]: https://github.com/foundry-rs/foundry/issues/4418
[revive]: https://github.com/foundry-rs/foundry/issues/10021
[core-fe]: https://github.com/foundry-rs/foundry-core/pull/206
[foundry-fe]: https://github.com/foundry-rs/foundry/pull/16968
[compiler-traits]: https://github.com/foundry-rs/foundry-core/blob/1e313aa20ae3185eabafdaa7f0dac22d277f0348/crates/compilers/crates/compilers/src/compilers/mod.rs
[fe-projects]: https://fe-lang.org/ingots/project-structure/
[fe-workspaces]: https://fe-lang.org/ingots/workspaces/
[fe-metadata]: https://blog.fe-lang.org/posts/release-26-2/
