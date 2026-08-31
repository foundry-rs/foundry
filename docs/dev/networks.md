# Custom EVM integrations

Foundry supports execution families whose transaction types, hardforks, precompiles, or state differ
from Ethereum. An integration crosses Foundry, Alloy, and REVM, so it must define ownership at each
layer and preserve that contract across Forge, Cast, Anvil, Chisel, scripts, cheatcodes, and traces.

## Mental model

Compile-time support and runtime selection are separate:

- Cargo features make optional network code available in a binary. Enabling a feature does not
  activate that network.
- [`NetworkConfigs`](../../crates/evm/networks/src/lib.rs) records the runtime execution profile.
  It is populated by `foundry.toml`, CLI flags, a namespaced hardfork, or fork endpoint discovery.
- [`FoundryEvmNetwork`](../../crates/evm/core/src/evm/mod.rs) associates an Alloy `Network` with a
  [`FoundryEvmFactory`](../../crates/evm/core/src/evm/mod.rs). Tool entry points dispatch to a
  concrete implementation only after runtime selection.

An optional RPC capability probe must not become a requirement for an otherwise valid endpoint.
Before an endpoint has positively identified a custom execution family, failed optional probes
should fall back to mandatory standard RPC discovery. Once the endpoint has been identified,
subsequent identity checks can remain strict.

## Ownership boundaries

| Layer | Owns |
| --- | --- |
| Alloy `Network` | RPC transaction requests, envelopes, receipts, and response types |
| Alloy `EvmFactory` and Foundry's `FoundryEvmFactory` adapter | Spec, block and transaction environments, halt reasons, precompiles, and construction of the concrete EVM context |
| REVM context and journal | Execution-time state mutation, checkpoints, commits, and reverts |
| `NetworkConfigs` and hardfork types | Runtime selection, chain and endpoint inference, feature configuration, and hardfork validation |
| Foundry tools | CLI/config plumbing, dispatch, forks, scripts, cheatcodes, tracing, verification, and user-visible behavior |

Do not encode protocol behavior only as a chain-ID branch in a tool. Put execution semantics in the
network factory or context, selection in the network configuration layer, and tool-specific workflow
behavior in the relevant tool.

## Adding an execution family

Start by writing down which parts differ from Ethereum: RPC envelopes, transaction validation,
hardfork schedule, block environment, precompiles, system calls, fee accounting, or auxiliary
state. Then implement the integration in layers.

1. Add hardfork parsing and activation rules under `crates/evm/hardforks`.
2. Add the runtime family and its configuration to `crates/evm/networks`. Define explicit selection,
   known-chain inference, endpoint metadata, compatibility with fork sources, and legacy aliases if
   required.
3. Add the concrete `FoundryEvmNetwork` and `FoundryEvmFactory` adapter under
   `crates/evm/core/src/evm/`. Keep conversions between RPC types and execution types at this
   boundary.
4. Thread the associated spec, block, transaction, halt, context, and journal types through the EVM
   backend. Avoid converting back to Ethereum types in generic paths.
5. Add runtime dispatch to every affected entry point. Search for existing `NetworkVariant` and
   `FoundryEvmNetwork` matches in Forge, Cast, Anvil, Chisel, scripting, verification, tracing, and
   cheatcodes.
6. Feature-gate optional dependencies consistently through every crate and binary that reaches the
   integration. A default build, a build with the feature, and the published release build are three
   distinct configurations.

Large integrations should be split into reviewable layers when possible: hardfork and configuration,
core execution, individual tool surfaces, then CI and documentation. Each layer should retain working
non-custom execution paths.

## State lifecycle

Custom execution state is often not fully represented by ordinary account storage. For every piece
of auxiliary state, document:

- where it is initialized for local execution and remote forks;
- whether it belongs to the database, chain context, block context, transaction context, or journal;
- how it is cloned for nested EVMs and isolation mode;
- when it is committed or discarded after success, revert, or halt;
- how snapshots, `revertToState`, state dumps, and Anvil rollback restore it;
- how fork creation, selection, rolling, reset, and RPC replacement reinitialize it;
- how `vm.transact`, script simulation, calls, estimates, access lists, traces, and historical replay
  obtain the correct block and transaction context;
- which caches derive from it and how those caches are invalidated.

If state must survive a nested execution, implement the transfer explicitly at the factory, context,
or journal boundary. Do not rely on cloning the ordinary account database to preserve state owned by
another component.

## Tool coverage

An integration is incomplete until the supported user workflows dispatch to the same execution
family and hardfork semantics. Audit at least:

- `forge test`, coverage, fuzz replay, gas reports, and inline configuration;
- `forge script`, simulation, broadcasting, resuming, and contract creation;
- `cast call`, `cast run`, transaction decoding, tracing, and gas estimation;
- Anvil startup, mining, RPC calls, fork startup/reset/roll, snapshots, dumps, and replay;
- Chisel startup, cached sessions, and forks;
- cheatcode execution, nested EVMs, isolation mode, and network-specific cheatcode addresses;
- trace decoding, labels, verification, and contract-size or fee rules.

Unsupported combinations should fail at the selection boundary with a specific error. Ordinary
Ethereum and other enabled families must continue to use their existing path.

## Tests and CI

Use a layered test plan:

1. Unit-test hardfork parsing, network selection, transaction conversion, and factory behavior.
2. Add conformance-style tests for auxiliary state across commit, revert, nested execution,
   snapshots, fork operations, and historical replay.
3. Add hermetic CLI integration tests for each supported tool. Prefer in-process nodes and local RPC
   proxies over live providers.
4. Exercise explicit selection, inferred selection, an unknown chain, and a conflicting or disabled
   family. Test optional endpoint probes independently from mandatory RPC methods.
5. Compile the workspace without optional features, with each feature independently, and with the
   release feature set. Verify that enabling one family does not change another family's execution.
6. Add the focused library and integration suites to normal pull-request CI. Do not rely only on a
   downstream repository or a nightly flaky-test job for release-critical behavior.

Keep feature propagation synchronized across workspace manifests, the root `Makefile`, and the
release and Docker workflows. Tests that use forks must contain `fork` in their names.

## Documentation and releases

User-facing selection, configuration, and workflows belong in the
[Foundry Book](https://getfoundry.sh). CLI option text belongs in the Clap definitions and is
generated into the book. Trait, context, and state invariants belong in Rustdoc next to their
implementation. Cross-crate integration guidance belongs here.

Add a changelog fragment for user-visible behavior. If a change only reorganizes contributor
documentation or CI and has no release-facing effect, use the repository's `L-ignore` label instead
of inventing a package release note.
