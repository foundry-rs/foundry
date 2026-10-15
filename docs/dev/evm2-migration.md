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
| Lifecycle | evm2 replaces its spec and `Version` (which carries the chain ID) only between transactions. | A fork switch or snapshot revert inside a transaction applies state and block at once and the spec or chain ID at the next transaction (`EnvTransition::Deferred`). | `Evm::set_execution_config` during execution. |
| Lifecycle | A fork switch accepts the running transaction's writes. | Like the REVM backend, an enclosing revert does not undo writes made before a fork switch. | None needed. |
| Transactions | evm2 frames borrow the transaction environment (origin, gas price, chain ID) for the whole transaction. | Cheatcodes that change these mid-transaction apply to the next transaction. | Mutable `TxEnv` for nested frames. |
| OP | op-evm2 has no post-execution (`0x7D`) transactions. | Converting one fails with an explicit error. | op-evm2 PostExec handler. |
| Base | base-evm2 registers no EIP-8130 handler; stateful Beryl/Cobalt precompiles are missing. | Executing an EIP-8130 transaction fails as an unsupported transaction type. | base-evm2 handlers. |
| Tempo | Foundry's RPC types and Tempo's evm2 integration pin different Tempo revisions. | Envelopes cross over as their exact EIP-2718 encoding. | Single Tempo pin. |
| Fork cache | `foundry-fork-db` stores REVM-typed accounts. | `ForkDb` converts at the read boundary. | evm2-typed fork cache in `foundry-core`. |
| Replay | Celo CIP-64 fee-currency transactions have no evm2 transaction type. | Replaying or `vm.transact`-ing one fails with an explicit error. | evm2 Celo transaction type. |
| Replay | The Celo transfer precompile and the BSC P256 precompile are not installed. | Fork prefix replay and `vm.transact` run with the network family's standard precompiles; transactions that call them diverge from chain. | Network precompile injection in `FoundryEvmFactory::precompiles`. |
| Replay | Protocol system transactions (known system senders, except `0x7E` deposits) have no native representation; Monad protocol system calls included. | They are skipped when replaying a block prefix, so state they write is missing. | Family system-call handlers. |
| Monad | Synthetic transactions on a fork (`transact_from_tx`, calls) do not reconstruct the block-position context. | They prepare Monad's reserve-balance context with `FoundryEvmExt::for_transaction`; `vm.transact` and prefix replay use the exact block context. | Track the fork's block position in `ForkInfo`. |
| Backend | `refresh_fork_storage` cannot override a slot the running transaction already loaded. | The refreshed value is set as a journaled write, so its original value (for `SSTORE` gas and refunds) stays the pre-refresh value. | evm2 slot override, like `AccountHandle::override_balance`. |
| Backend | The local database is evm2's `EmptyDB`, which reports unknown accounts as absent. | REVM's `EmptyDBWrapper` reported them as empty existing accounts; only pre-Spurious Dragon execution can observe the difference. | None needed. |
| Hardforks | `foundry-evm-hardforks` keeps the REVM spec types (`SpecId`, `OpSpecId`, `TempoHardfork`, `MonadHardfork`, `BaseSpecId`) for hardfork name parsing and activation schedules. | evm2 spec types implement `ExecutionSpec` by delegating to them by name; `tempo` precompile activation in `foundry-evm-networks` maps the evm2 Tempo hardfork to the REVM one by name. | Port the name tables and schedules to the evm2 types. |
| Anvil storage | Anvil's state dumps and forked database use REVM account types. | `backend::{MemDb, FoundryEvmInMemoryDB, EmptyDBWrapper, StateSnapshot}` and `fork::database` stay REVM-typed in `foundry-evm-core` until Anvil is ported. | Anvil layer. |

## Dependencies

evm2 is patched workspace-wide onto `mablr/evm2@mablr/foundry-lifecycle`, which adds the
suspended-state lifecycle API (`finalize_transaction`, warmth restoration, `set_prewarm_set`) on
top of the revision Tempo and `evm2-networks` share. Keep the three in lockstep.
