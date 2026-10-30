# evm2 migration

Foundry executes every supported network family (Ethereum, OP, Base, Monad and Tempo) on
[evm2](https://github.com/alloy-rs/evm2). This document records the cross-crate contract of that
execution stack and the known parity gaps. Crate-level details live in Rustdoc, starting at
[`foundry_evm_core::evm`](../../crates/evm/core/src/evm/mod.rs).

## Ownership

```text
┌───────────────────────────────────────────────────────────────┐
│ Forge / Cast / Anvil / Chisel / Script                        │
│ select a FoundryEvmNetwork once; keep native Alloy requests   │
│ and signed envelopes until execution                          │
└──────────────────────────────┬────────────────────────────────┘
                               │
┌──────────────────────────────▼────────────────────────────────┐
│ Backend<FEN> (foundry-evm-core)                               │
│ ExecutionLifecycle: forks, persistent accounts, snapshots,    │
│ nested frames and nested transactions; RPC fork creation;     │
│ cheatcode access policy                                       │
└───────────────┬──────────────────────────────┬────────────────┘
                │                              │
┌───────────────▼──────────────┐ ┌─────────────▼─────────────────┐
│ evm2::Evm<FEN types>         │ │ BackingDb                     │
│ State: tx scratch, journal,  │ │ local in-memory or ForkDb     │
│ warmth, logs, accepted cache │ │ over the RPC fork cache       │
└──────────────────────────────┘ └───────────────────────────────┘
```

- evm2 `State` owns transaction scratch, journaling, warmth, transient storage, logs and the
  running context's accepted overlay.
- `BackingDb` only serves reads that miss the overlay. It never receives writes.
- `ExecutionLifecycle` owns everything Foundry layers on top: the accepted state of inactive forks,
  persistent accounts, user-visible snapshots and nested execution. It replaces the REVM-era
  `DatabaseExt`.
- Transactions stay network-native. `FoundryEvmNetwork::{tx_from_request, tx_from_signed,
  decode_tx}` convert once, right before execution.

## Execution

`Backend::create_evm` builds an `evm2::Evm` over the active context. The EVM does not borrow the
backend: it reads the context's accepted state through a shared copy-on-write view, so the backend
stays free while the EVM runs. Inspectors that implement cheatcodes hold `&mut Backend` (or a
`CowBackend`) for the duration of the execution and pass the `&mut Evm` they receive from their
hooks to the backend operations that act on the running transaction (fork selection and rolls,
snapshots, `vm.transact`, allocs, account refreshes), together with the `ExecutionPhase`.
`Backend::park` accepts a committed EVM's state, environment and network state back into the
active context; dropping the EVM discards it, as for calls. `Backend::inspect_call` and
`Backend::inspect_commit` wrap these steps for executions whose inspector needs no backend access.

## Known parity gaps

Gaps are recorded here when evm2 or an adapter cannot express current behavior. Each one fails
explicitly or is documented as different; none falls back to REVM.

| Area | Gap | Behavior | Extension point |
| --- | --- | --- | --- |
| Arbitrary storage | Master reads every slot warm after `vm.setArbitraryStorage`. | Accepted difference, a master warmth artifact: slots of an arbitrary-storage account warm on first access as usual, so the first read of a slot costs 2,000 more than on master. | None. |
| `vm.transact` | Master runs `vm.transact` over a fresh backend, which denies cheatcodes to `vm.allowCheatcodes` accounts. | Accepted difference: inside `vm.transact`, accounts allowed with `vm.allowCheatcodes` can call cheatcodes, as in other calls. | None. |
| Amsterdam isolation | Master never sets the nonce of an isolated call's transaction, so isolated `CREATE`s after the first skip EIP-8037 account-creation state gas. | Accepted difference: master undercharges them; every isolated `CREATE` pays state gas, as a non-isolated one does. | None. |
| `vm.setEvmVersion` | Master charges a test's intrinsic gas at the configured spec, but computes the stipend it subtracts from a revm `CfgEnv` whose EIP-2780 flag stays set once any call switched to Amsterdam. | Accepted difference, a master gas-report artifact: after `vm.setEvmVersion("amsterdam")` in `setUp`, master reports 6,000 more gas per test, even after switching back. evm2 changes the spec only between frames, so a change inside a frame applies from its next call frame, not from the next opcode as on master. | `Evm::set_execution_config` during execution. |
| Fork warmth across transactions | Master keeps a journal per fork across transactions, so storage and accounts touched in `setUp` stay warm after `vm.selectFork` or `vm.rollFork` in a test; with `isolate`, its `vm.rollFork` instead cools slots the test already read. | Accepted difference, master warmth artifacts: warmth is per transaction and survives fork switches inside it in both isolation modes (`Issue3110`, `Issue3220`: +2,000; `Issue5739`, `Issue8006` and `PersistentWrites` with `isolate`: −2,000 to −2,500). | None. |
| Anvil precompiles | `PrecompileFactory` returns a fixed `(Address, PrecompileId, DynPrecompile)` list; REVM's dynamic precompile lookup (`install`) has no equivalent. | Library users must list precompile addresses up front. | evm2 lookup-based precompile provider. |
| Dependencies | `foundry-evm-hardforks`, `foundry-evm-networks`, `foundry-common-fmt` and the native fork cache depend on no REVM crate, but external crates still compile REVM (`revm`, `alloy-evm`) transitively: `tempo-evm` (evm2 integration) through `reth-evm`, `reth-evm-ethereum` and `reth-consensus`; `base-consensus-upgrades`, `base-protocol`, `base-common-chains`, `base-common-network`, `base-common-rpc-types` and `base-common-precompiles` (`foundry-evm-traces`); `base-common-consensus` with its `evm` feature, which `foundry-evm-core` enables, so `base-evm2` and `base-common-eip8130` pull REVM in unified builds. Foundry retains REVM's serialized fork block metadata type. | REVM is compiled but not executed. | REVM-free releases of these crates, or dropping the `evm` features that enable it. |

## Dependencies

The evm2 and evm2-networks pins, and why the root manifest patches them, are documented in the
root `Cargo.toml`. `mark_created_keeping_storage` keeps an account created for EIP-6780 when a
suspended or isolated transaction continues on a fresh overlay, without reading its storage as
discarded. Foundry enables `EIP7702` on every spec so that already-present delegation code resolves
before Prague, matching REVM; authorization processing remains Prague-gated by the transaction
registry.

[`foundry-core@mablr/fork-db-evm2-completion`](https://github.com/mablr/foundry-core/tree/mablr/fork-db-evm2-completion)
stores native evm2 accounts and bytecode. Existing JSON/zstd caches remain readable, including
legacy analyzed bytecode and EIP-7702 delegation. RPC cache reads remain separate from execution
commits. [`evm2-networks`](https://github.com/foundry-rs/evm2-networks) provides the network adapters.

Every Tempo crate uses tempoxyz/tempo#7871 directly, which brings upstream reth and reth-core.
Root patches do not propagate, so the root manifest repeats #7871's reth-core `[patch.crates-io]`
entries and its evm2 patch. Anvil overrides EIP-7702 authorities for Tempo by rebuilding the
transaction environment from the overridden Ethereum transaction, so no Tempo fork is needed.

## Completion follow-ups

Dependency readiness is not full Foundry parity. Do not bless failing snapshots or weaken
validation to hide these differences. The existing tests remain executable reproducers.

- OP `0x7D` needs block-position and earlier-receipt inputs. Monad staking system-call helpers
  already exist, but protocol transaction replay still needs an authoritative codec and block
  fixtures. Do not register ordinary or no-op handlers for either protocol transaction kind.

Blob simulation disables `BLOB_BASE_FEE_CHECK` only for zero-cap calls without validation;
explicit caps still validate and pay the simulated blob fee. `BASE_FEE_CHECK` remains independent,
as in REVM. The updated wallet dependency removes the old `time = 0.3.47` constraint, allowing
Anvil's `js-tracer` feature to use evm2-inspectors/Boa directly. Isolated pre-London execution uses
legacy transactions rather than unsupported EIP-1559 transactions. Monad's block participant
tracking uses the same local authority-recovery overrides as execution, including replay and
persisted block metadata.
