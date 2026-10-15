# evm2 migration

Foundry executes every supported network family (Ethereum, OP, Base, Monad and Tempo) on
[evm2](https://github.com/alloy-rs/evm2). This document records the cross-crate contract of that
execution stack and the known parity gaps. Crate-level details live in Rustdoc, starting at
[`foundry_evm_core::evm`](../../crates/evm/core/src/evm2/mod.rs).

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

An execution moves the backend into the inspector stack for its duration, so cheatcodes reach the
lifecycle through the stack while `evm2::Evm` runs, and takes it back afterwards. Committing
parks the EVM's accepted state back into the active context; calls discard it.

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
| Fuzz | The evm2 accepted overlay holds only committed changes, while REVM's fork `CacheDB` also cached every account `setUp` read. | `EvmFuzzState::new` seeds the dictionary from accounts, code and storage `setUp` accepted; accounts it only read from a fork are not seeded. | Seed from the fork cache as well. |
| Fuzz | evm2 reports account changes, not REVM's touched flag. | Contracts are discovered from accounts a call changed; an account it only called without changing is not rediscovered. | None needed. |

## Dependencies

evm2 is patched workspace-wide onto `mablr/evm2@mablr/foundry-lifecycle`, which adds the
suspended-state lifecycle API (`finalize_transaction`, warmth restoration, `set_prewarm_set`) on
top of the revision Tempo and `evm2-networks` share. Keep the three in lockstep.
