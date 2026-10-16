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
| Transactions | evm2 frames borrow the transaction environment (origin, gas price, chain ID, blob hashes) for the whole transaction. | `vm.prank` with origin, `vm.txGasPrice`, `vm.blobhashes` and `vm.chainId` inside a transaction override the `ORIGIN`, `GASPRICE`, `BLOBHASH` and `CHAINID` opcodes at once; fee accounting keeps the transaction's values, and the chain ID applies to validation from the next transaction (`EnvTransition::Deferred`). | Mutable `TxEnv` for nested frames. |
| Cheatcodes | `vm.transact` and `vm.broadcastRawTransaction` execute through `Backend::transact*`, which cannot re-enter the cheatcode inspector that drives them. | The replayed transaction runs with the stack's tracing, logging and other non-cheatcode inspectors; REVM also ran the cheatcode inspector inside it. | Re-entrant inspector access from `Backend::transact*`. |
| Cheatcodes | evm2's `BlockEnv` has no excess blob gas. | `vm.blobBaseFee` sets the blob base fee from `fake_exponential`; `vm.getBlobBaseFee` returns the last value set, which snapshots and fork switches do not revert. | Excess blob gas on `BlockEnv`. |
| Cheatcodes | evm2 has no API to mark a warm account cold. | `vm.cool` cools the account's loaded storage slots but not the account. | `State::set_account_cold`. |
| Cheatcodes | evm2 does not expose accounts and slots that were only loaded. | `vm.dumpState`, `vm.cloneAccount` and `vm.copyStorage` see the accepted overlay plus the running transaction's journal; accounts or slots that were only read are omitted. | Loaded-state iteration on `State`. |
| Cheatcodes | evm2 has no re-entrant host call from a storage hook. | Storage hook callbacks run on a separate frame EVM with warmth restored afterwards. | None needed. |
| Cheatcodes | A snapshot revert or fork switch inside an isolated call replaces state the parent frame cannot merge. | The parent adopts the isolated child's whole state. | None needed. |
| Cheatcodes | Monad's context snapshots and the isolated-snapshot journal tracking relied on REVM journals. | Snapshots use `Backend::snapshot_state`/`revert_state`; logs from `vm.executeTransaction` are not merged into the receipt (as before). | None needed. |
| Isolation | evm2 has no per-call transaction boundary inside a running transaction. | With `isolate`, each top-level call runs as a nested fee-less transaction from the call's caller and nonce with the call's gas plus intrinsic gas, raised to the EIP-7623 floor and capped at the block gas limit; the parent frame is charged its execution and state gas and gets its refund on success. EIP-8037 state gas the parent precharged is not credited, cheatcode precompile flags do not apply, and without cheatcodes `ORIGIN` is the call's caller. | Nested transactions in the evm2 host. |
| Executor | `call_tx`, `transact_tx` and block replay take converted network transactions, whose request fields are gone. | Their results report a zero `stipend`; `calculate_stipend` on a request omits EIP-7702 authorization costs. | None needed. |
| Tempo | Tempo's EVM extension shares transaction-local state (storage action buffers, non-creditable slots) with its precompiles through `Rc`, which cannot cross threads. | Parked contexts and snapshots store a detached copy (`StoredExt`) that keeps fee settings and resets that transaction-local state. | Thread-safe Tempo extension state. |
| OP | op-evm2 has no post-execution (`0x7D`) transactions. | Converting one fails with an explicit error. | op-evm2 PostExec handler. |
| Base | base-evm2 registers no EIP-8130 handler; stateful Beryl/Cobalt precompiles are missing. | Executing an EIP-8130 transaction fails as an unsupported transaction type. | base-evm2 handlers. |
| Tempo | Foundry's RPC types and Tempo's evm2 integration pin different Tempo revisions. | Envelopes cross over as their exact EIP-2718 encoding. | Single Tempo pin. |
| Fork cache | `foundry-fork-db` stores REVM-typed accounts. | `ForkDb` converts at the read boundary. | evm2-typed fork cache in `foundry-core`. |
| Fuzz | The evm2 accepted overlay holds only committed changes, while REVM's fork `CacheDB` also cached every account `setUp` read. | `EvmFuzzState::new` seeds the dictionary from accounts, code and storage `setUp` accepted; accounts it only read from a fork are not seeded. | Seed from the fork cache as well. |
| Fuzz | evm2 reports account changes, not REVM's touched flag. | Contracts are discovered from accounts a call changed; an account it only called without changing is not rediscovered. | None needed. |
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
