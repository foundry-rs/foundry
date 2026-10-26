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
| Lifecycle | Fork warmth is kept per context, not in one running journal. | After an isolated call that switches forks and succeeds, selecting the original fork back costs 537 gas in total where master's `selectFork` cost 2,537, as master read the fork ID slot cold. An untouched warm account later made persistent arrives cold on the other fork, where master had it warm. | None needed. |
| Lifecycle | A reverted fork switch does not replay REVM's per-account journal. | After an isolated call switches forks and reverts and the original fork is selected back, the test contract's account is warm (110 gas) where master had it cold (2,610). | None needed. |
| Lifecycle | Cheatcode storage reads are not journaled account accesses. | Accounts touched only through `vm.load` around a fork switch are cold (2,610 gas) after it, where master had them warm (110). | None needed. |
| Isolation | Isolated static calls cool only the running state. | In isolate mode, a top-level `STATICCALL` cools the active state but leaves the warmth parked with inactive forks, so a later fork round trip finds the fork ID slot warm, 2,000 gas below master, which cooled it. | None needed. |
| Cheatcodes | Replays on an inactive fork rebase the running transaction's loaded accounts as journaled writes. | Like master, `vm.transact` and `vm.rollFork` to a transaction on an inactive fork replace the active fork's loaded non-persistent accounts with the inactive fork's resulting state (`monad_fork_aux_lifecycle_tracks_outer_context`). Unlike master, which reloaded them into its journal without revert entries, the rebased values revert with an enclosing frame that reverts. | None needed. |
| Cheatcodes | evm2 has no journaled storage wipe. | A nested transaction merged as journaled writes (`vm.transact` on the active fork, isolated calls, the persistent accounts of an inactive-fork replay) that wipes an account's storage, such as a pre-Cancun `SELFDESTRUCT`, marks the wipe with `StorageHandle::wipe`, which evm2 does not journal. An enclosing revert restores the account's info but leaves its storage wiped. Per-slot journaled clears cannot reach slots the running transaction never loaded, and `override_wipe` survives rollback. Untracked merges (`vm.executeTransaction`, `vm.broadcastRawTransaction`) are unaffected. | Journaled storage wipe on `StorageHandle`. |
| Cheatcodes | Replayed transactions merge account changes, not fork-context transitions. | `vm.transact` and `vm.broadcastRawTransaction` run with the full inspector stack, including cheatcodes; `vm.selectFork`, `vm.rollFork` and `vm.createSelectFork` fail explicitly during replay. | Fork-context handoff from replayed transactions. |
| Cheatcodes | Transaction-base warmth is separate from runtime warmth. | `vm.cool` cools loaded accounts and storage slots, except entries in the transaction-base pre-warmed set. | Cheatcode-specific pre-warmed-set removal. |
| Isolation | evm2 has no per-call transaction boundary inside a running transaction. | With `isolate`, each top-level call runs as a nested fee-less transaction from the call's caller and nonce with the call's gas plus intrinsic gas, raised to the EIP-7623 floor and capped at the block gas limit; the parent frame is charged its execution and state gas and gets its refund on success. EIP-8037 state gas the parent precharged is not credited, cheatcode precompile flags do not apply, and without cheatcodes `ORIGIN` is the call's caller. | Nested transactions in the evm2 host. |
| Executor | `call_tx`, `transact_tx` and block replay take converted network transactions, whose request fields are gone. | Their results report a zero `stipend`; `calculate_stipend` on a request omits EIP-7702 authorization costs. | None needed. |
| Tempo | Tempo's EVM extension shares transaction-local state (storage action buffers, non-creditable slots) with its precompiles through `Rc`, which cannot cross threads. | Parked contexts and snapshots store a detached copy (`StoredExt`) that keeps fee settings and resets that transaction-local state. | Thread-safe Tempo extension state. |
| Fork cache | RPC endpoints report absent accounts as empty, so `ForkDb` cannot tell an existing empty account from an absent one. | `ForkDb` follows the EIP-161 rule of the latest EVM built or environment installed over the fork: with EIP-161 empty remote accounts load as absent, before it as existing (as REVM loaded them), so pre-Spurious Dragon execution sees absent empty accounts as existing. | None needed. |
| Fuzz | Committed-session read observations are retained in the per-context cache, not just writes. | `EvmFuzzState::new` seeds accounts, code and storage from that cache after `setUp`, including read-only fork observations. Snapshot restoration drops observations obtained after the snapshot. The shared RPC cache is not a discovery source. | None needed for committed-session reads. |
| Symbolic | Committed-session read observations are retained in the per-context cache, not just writes. | Symbolic calls to a symbolic address consider accounts cached by `setUp`, including read-only fork observations, and accounts the symbolic run itself loaded. Snapshot restoration drops observations obtained after the snapshot. The shared RPC cache is not a discovery source. | None needed for committed-session reads. |
| Replay | Protocol system transactions of known system senders, other than `0x7E` deposits and canonical Monad protocol system calls, have no native representation. | They are skipped when replaying a block prefix, so state they write is missing. Canonical Monad protocol system calls replay through `FoundryEvmExt::set_protocol_replay` in trusted historical execution only (prefix replay, `vm.transact`, `cast run`, `verify-bytecode`); a non-canonical envelope from the Monad system sender is skipped in a prefix and rejected as a target. | Family system-call handlers. |
| Script | evm2 transactions are opaque, so a converted signed transaction cannot have its fees zeroed. | Fixed-gas simulation of a signed transaction executes the request rebuilt from its envelope (same type, calls, access and authorization lists) with zero fees, priced as a simulation: the OP-stack L1 data fee is not charged. Broadcast and `--resume` still send and decode exactly the persisted signed bytes. | None needed. |
| Verify | `verify-bytecode` replaces the creation transaction's input with the local creation code. | The target executes as the request rebuilt from its on-chain envelope with the new input, priced as a simulation; prefix transactions execute as signed. On Monad, canonical protocol system transactions in the prefix replay as protocol system calls. | None needed. |
| Cast run | Block replay skips prefix transactions from known system senders other than canonical Monad protocol system calls. | `cast run` of a canonical Monad protocol system transaction replays it as a protocol system call, and a non-canonical envelope from the Monad system sender fails; `--replay-system-txes` still admits other system transaction targets and `0x7E` prefix transactions, but no longer replays other known-system-sender prefix transactions. | Family system-call handlers. |
| Backend | The local database is evm2's `EmptyDB`, which reports unknown accounts as absent. | REVM's `EmptyDBWrapper` reported them as empty existing accounts; only pre-Spurious Dragon execution can observe the difference. | None needed. |
| Anvil precompiles | `PrecompileFactory` returns a fixed `(Address, PrecompileId, DynPrecompile)` list; REVM's dynamic precompile lookup (`install`) has no equivalent. | Library users must list precompile addresses up front. | evm2 lookup-based precompile provider. |
| Dependencies | `foundry-evm-hardforks`, `foundry-evm-networks`, `foundry-common-fmt` and the native fork cache depend on no REVM crate, but external crates still compile REVM (`revm`, `alloy-evm`) transitively: `tempo-evm` (evm2 integration) through `reth-evm`, `reth-evm-ethereum` and `reth-consensus`; `base-consensus-upgrades`, `base-protocol`, `base-common-chains`, `base-common-network`, `base-common-rpc-types` and `base-common-precompiles` (`foundry-evm-traces`); `base-common-consensus` with its `evm` feature, which `foundry-evm-core` enables, so `base-evm2` and `base-common-eip8130` pull REVM in unified builds. Foundry retains REVM's serialized fork block metadata type. | REVM is compiled but not executed. | REVM-free releases of these crates, or dropping the `evm` features that enable it. |

## Dependencies

Foundry and its satellites share
[`mablr/evm2@mablr/foundry-completion`](https://github.com/mablr/evm2/tree/mablr/foundry-completion).
It merges alloy-rs/evm2 up to `29690bd`, the newest revision tempoxyz/tempo#7871 still compiles
against (evm2#785 changes `GasParams::get`), and adds suspended-state lifecycle APIs
(`finalize_transaction`, warmth restoration, `cool_loaded_state`, `set_prewarm_set`,
`mark_created_keeping_storage`), independent `BLOB_BASE_FEE_CHECK` policy, account warmth control
from evm2#705, and resolution of already-present delegation code before Prague, matching REVM.
`mark_created_keeping_storage` keeps an account created for EIP-6780 when a suspended or isolated
transaction continues on a fresh overlay, without reading its storage as discarded.
Authorization processing remains Prague-gated. The lockfile must contain one evm2 graph, not
separate engines for different networks. Tempo and reth pin `alloy-rs/evm2`, while the satellites
pin `mablr/evm2`; the root manifest patches both to that branch through the `DaniPopes/evm2` URL,
since Cargo rejects a patch from the same source.

[`foundry-core@mablr/fork-db-evm2-completion`](https://github.com/mablr/foundry-core/tree/mablr/fork-db-evm2-completion)
stores native evm2 accounts and bytecode. Existing JSON/zstd caches remain readable, including
legacy analyzed bytecode and EIP-7702 delegation. RPC cache reads remain separate from execution
commits. [`evm2-networks`](https://github.com/mablr/evm2-networks) provides the network adapters.

Every Tempo crate uses tempoxyz/tempo#7871 directly, which brings upstream reth and reth-core.
Root patches do not propagate, so the root manifest repeats #7871's reth-core `[patch.crates-io]`
entries and its evm2 patch. Anvil overrides EIP-7702 authorities for Tempo by rebuilding the
transaction environment from the overridden Ethereum transaction, so no Tempo fork is needed.

## Completion follow-ups

Dependency readiness is not full Foundry parity. Do not bless failing snapshots or weaken
validation to hide these differences. The existing tests remain executable reproducers.

- `Issue6032.t.sol` and `Fork2.t.sol`: distinguish transaction-loaded state, per-context cached
  observations and accepted writes from the shared RPC cache. Fuzz and symbolic setup discovery
  already uses the per-context cache, which includes committed-session reads. Do not enumerate
  the shared RPC cache: it can contain persisted or sibling-execution reads. Remaining fork-switch
  visibility needs separate lifecycle semantics.
- OP `0x7D` needs block-position and earlier-receipt inputs. Monad staking system-call helpers
  already exist, but protocol transaction replay still needs an authoritative codec and block
  fixtures. Do not register ordinary or no-op handlers for either protocol transaction kind.
- Amsterdam isolation still needs state-gas accounting across the nested transaction boundary.
  Use existing gas regression cases and compare parent precharge, child execution and refunds;
  do not adjust reported gas or golden values to compensate.
- `test_tempo_eip7702_replay_signature_override`: Tempo's converted `TempoTxEnv` exposes only
  immutable access to its `LazyTxEip7702`. A narrow upstream API for recovered-authority overrides
  would allow the same local impersonation policy as Ethereum, OP and Monad without changing
  signed transaction bytes or bypassing authorization validation.

Blob simulation disables `BLOB_BASE_FEE_CHECK` only for zero-cap calls without validation;
explicit caps still validate and pay the simulated blob fee. `BASE_FEE_CHECK` remains independent,
as in REVM. The updated wallet dependency removes the old `time = 0.3.47` constraint, allowing
Anvil's `js-tracer` feature to use evm2-inspectors/Boa directly. Isolated pre-London execution uses
legacy transactions rather than unsupported EIP-1559 transactions. Monad's block participant
tracking uses the same local authority-recovery overrides as execution, including replay and
persisted block metadata.
