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
| Lifecycle | evm2 replaces its spec and `Version` (which carries the chain ID) only between transactions. | A fork switch or snapshot revert inside a transaction applies state and block at once and the spec or chain ID at the next transaction (`EnvTransition::Deferred`), while `CHAINID` reads a switched fork's chain ID at once; `vm.setEvmVersion` also applies from the next transaction, such as from `setUp` to the test. | A journal-aware environment transition; bypassing evm2's execution guard is not sufficient. |
| Lifecycle | A fork switch accepts the running transaction's writes. | Like the REVM backend, an enclosing revert does not undo writes made before a fork switch. | None needed. |
| Lifecycle | A fork switch replaces the running state with the target fork's. | Accounts the running transaction created or loaded on the source fork are not visible on the target fork unless persistent (`Issue6032.t.sol`, `Fork2.t.sol`'s `testForkDumpState*`); REVM's journal kept them until the transaction ended. | None needed. |
| Lifecycle | Fork warmth is kept per context, not in one running journal. | After an isolated call that switches forks and succeeds, selecting the original fork back costs 537 gas in total where master's `selectFork` cost 2,537, as master read the fork ID slot cold. An untouched warm account later made persistent arrives cold on the other fork, where master had it warm. | None needed. |
| Lifecycle | A reverted fork switch does not replay REVM's per-account journal. | After an isolated call switches forks and reverts and the original fork is selected back, the test contract's account is warm (110 gas) where master had it cold (2,610). | None needed. |
| Lifecycle | Cheatcode storage reads are not journaled account accesses. | Accounts touched only through `vm.load` around a fork switch are cold (2,610 gas) after it, where master had them warm (110). | None needed. |
| Isolation | Isolated static calls cool only the running state. | In isolate mode, a top-level `STATICCALL` cools the active state but leaves the warmth parked with inactive forks, so a later fork round trip finds the fork ID slot warm, 2,000 gas below master, which cooled it. | None needed. |
| Transactions | evm2 frames borrow the transaction environment (origin, gas price, chain ID, blob hashes) for the whole transaction. | `vm.prank` with origin, `vm.txGasPrice`, `vm.blobhashes` and `vm.chainId` inside a transaction override the `ORIGIN`, `GASPRICE`, `BLOBHASH` and `CHAINID` opcodes at once; fee accounting keeps the transaction's values, and the chain ID applies to validation from the next transaction (`EnvTransition::Deferred`). | Mutable `TxEnv` for nested frames. |
| Cheatcodes | Replayed transactions merge account changes, not fork-context transitions. | `vm.transact` and `vm.broadcastRawTransaction` run with the full inspector stack, including cheatcodes; `vm.selectFork`, `vm.rollFork` and `vm.createSelectFork` fail explicitly during replay. | Fork-context handoff from replayed transactions. |
| Cheatcodes | Transaction-base warmth is separate from runtime warmth. | `vm.cool` cools loaded accounts and storage slots, except entries in the transaction-base pre-warmed set. | Cheatcode-specific pre-warmed-set removal. |
| Cheatcodes | evm2 does not expose an iterator over transaction-loaded accounts and slots. | `vm.dumpState` sees the accounts the running transaction changed or warmed and the accounts it created. `vm.cloneAccount`, `vm.copyStorage` and dumped storage filter known overlay/journal keys to slots loaded in the running transaction, excluding setup-only cache entries. Unjournaled account loads and loaded slots no longer present in either candidate source can still be omitted; the dump also does not keep accounts that a fork switch, fork roll or enclosing revert removed from the active fork's state. | Loaded-state iteration on `State`. |
| Cheatcodes | evm2 has no re-entrant host call from a storage hook. | Storage hook callbacks run on a separate frame EVM with warmth restored afterwards. | None needed. |
| Cheatcodes | A snapshot revert or fork switch inside an isolated call replaces state the parent frame cannot merge. | The parent adopts the isolated child's whole state. | None needed. |
| Cheatcodes | evm2 journals the changes of a snapshot revert as a replacement of the running state, not per write. | A snapshot revert inside a call that then reverts or halts is not undone, in isolated and non-isolated calls alike (`StateSnapshots.t.sol`'s `NestedRestoreFrameRevert*`, `SnapshotCapsulePolicy.t.sol`); REVM tracked restores in its journal and undid their state changes with the failing frame. | Journaled snapshot restores in the evm2 host. |
| Cheatcodes | evm2 has no untracked (non-journaled) state writes. | Balance and nonce writes of `vm.deal`, `vm.setNonce`, `vm.setNonceUnsafe` and `vm.resetNonce`, and the changes of `vm.executeTransaction` and `vm.broadcastRawTransaction`, revert with an enclosing frame that reverts (`RevertedFrameEffects.t.sol`, `ExecuteTransactionState.t.sol`, `BroadcastRawTransaction.t.sol`); REVM wrote them without journal entries, so they survived it. | Untracked writes on `State`. |
| Cheatcodes | Monad's context snapshots and the isolated-snapshot journal tracking relied on REVM journals. | Snapshots use `Backend::snapshot_state`/`revert_state`; logs from `vm.executeTransaction` are not merged into the receipt (as before). | None needed. |
| Isolation | evm2 has no per-call transaction boundary inside a running transaction. | With `isolate`, each top-level call runs as a nested fee-less transaction from the call's caller and nonce with the call's gas plus intrinsic gas, raised to the EIP-7623 floor and capped at the block gas limit; the parent frame is charged its execution and state gas and gets its refund on success. EIP-8037 state gas the parent precharged is not credited, cheatcode precompile flags do not apply, and without cheatcodes `ORIGIN` is the call's caller. | Nested transactions in the evm2 host. |
| Isolation | evm2 uses a frame's message depth both for the call-depth limit and as the depth inspectors and cheatcodes see. | An isolated call keeps its depth in the surrounding transaction, so it can nest 1023 calls below a top-level call; REVM restarted the depth limit at the nested transaction and allowed 1024. | Separate call-depth limit base in the evm2 host. |
| Traces | evm2 fires `step_end` for call and create opcodes after the child frame returns, and steps code-less accounts through a single `STOP`. | Step traces (`forge test --json` at high verbosity, the debugger) report the gas a `CALL*`/`CREATE*` step consumed, child included, instead of the gas remaining after forwarding (REVM, geth struct logs), and show one `STOP` step in calls to accounts without code. | None needed. |
| Executor | `call_tx`, `transact_tx` and block replay take converted network transactions, whose request fields are gone. | Their results report a zero `stipend`; `calculate_stipend` on a request omits EIP-7702 authorization costs. | None needed. |
| Tempo | Tempo's EVM extension shares transaction-local state (storage action buffers, non-creditable slots) with its precompiles through `Rc`, which cannot cross threads. | Parked contexts and snapshots store a detached copy (`StoredExt`) that keeps fee settings and resets that transaction-local state. | Thread-safe Tempo extension state. |
| OP | op-evm2 has no post-execution (`0x7D`) transactions. | Converting one fails with an explicit error. | Block-level order/index validation and refund settlement over preceding receipts, not a no-op transaction handler. |
| Base | base-evm2 registers no EIP-8130 handler; B20, policy and TxContext precompiles are missing. | Executing an EIP-8130 transaction fails as an unsupported transaction type. Beryl tests that call the B20 factory revert (`test_cmd::base::base_beryl_precompiles_and_nested_evm`, `base_local_allows_stateful_precompile_writes`). The Cobalt nonce-manager read ABI is native. | Phased authorization, payer/nonce validation and settlement in base-evm2; read-only active `TxEnv` access for TxContext. |
| Tempo | Foundry's RPC types and Tempo's evm2 integration pin different Tempo revisions. | Envelopes cross over as their exact EIP-2718 encoding. | Single Tempo pin. |
| Fork cache | RPC endpoints report absent accounts as empty, so `ForkDb` cannot tell an existing empty account from an absent one. | `ForkDb` follows the EIP-161 rule of the latest EVM built or environment installed over the fork: with EIP-161 empty remote accounts load as absent, before it as existing (as REVM loaded them), so pre-Spurious Dragon execution sees absent empty accounts as existing. | None needed. |
| Fuzz | Committed-session read observations are retained in the per-context cache, not just writes. | `EvmFuzzState::new` seeds accounts, code and storage from that cache after `setUp`, including read-only fork observations. Snapshot restoration drops observations obtained after the snapshot. The shared RPC cache is not a discovery source. | None needed for committed-session reads. |
| Fuzz | evm2 reports account changes, not REVM's touched flag. | Contracts are discovered from accounts a call changed; an account it only called without changing is not rediscovered. | None needed. |
| Symbolic | Committed-session read observations are retained in the per-context cache, not just writes. | Symbolic calls to a symbolic address consider accounts cached by `setUp`, including read-only fork observations, and accounts the symbolic run itself loaded. Snapshot restoration drops observations obtained after the snapshot. The shared RPC cache is not a discovery source. | None needed for committed-session reads. |
| Replay | Celo CIP-64 fee-currency transactions have no evm2 transaction type. | Replaying or `vm.transact`-ing one fails with an explicit error. | evm2 Celo transaction type. |
| Replay | Protocol system transactions of known system senders, other than `0x7E` deposits and canonical Monad protocol system calls, have no native representation. | They are skipped when replaying a block prefix, so state they write is missing. Canonical Monad protocol system calls replay through `FoundryEvmExt::set_protocol_replay` in trusted historical execution only (prefix replay, `vm.transact`, `cast run`, `verify-bytecode`); a non-canonical envelope from the Monad system sender is skipped in a prefix and rejected as a target. | Family system-call handlers. |
| Monad | Synthetic transactions on a fork (`transact_from_tx`, calls) do not reconstruct the block-position context. | They prepare Monad's reserve-balance context with `FoundryEvmExt::for_transaction`; `vm.transact` and prefix replay use the exact block context. | Track the fork's block position in `ForkInfo`. |
| Lifecycle | evm2's runtime version uses `u64::MAX` both for "no EIP-7825 cap" before Osaka and for Foundry's disabled cap. | A disabled cap is kept across spec changes, so with `enable_tx_gas_limit` on a network whose default spec predates the cap (OP-stack, Base), resolving to a later spec keeps the cap disabled. | A separate tx gas cap policy flag on `Version`. |
| Script | evm2 transactions are opaque, so a converted signed transaction cannot have its fees zeroed. | Fixed-gas simulation of a signed transaction executes the request rebuilt from its envelope (same type, calls, access and authorization lists) with zero fees, priced as a simulation: the OP-stack L1 data fee is not charged. Broadcast and `--resume` still send and decode exactly the persisted signed bytes. | None needed. |
| Verify | `verify-bytecode` replaces the creation transaction's input with the local creation code. | The target executes as the request rebuilt from its on-chain envelope with the new input, priced as a simulation; prefix transactions execute as signed. On Monad, canonical protocol system transactions in the prefix replay as protocol system calls. | None needed. |
| Cast run | Block replay skips prefix transactions from known system senders other than canonical Monad protocol system calls. | `cast run` of a canonical Monad protocol system transaction replays it as a protocol system call, and a non-canonical envelope from the Monad system sender fails; `--replay-system-txes` still admits other system transaction targets and `0x7E` prefix transactions, but no longer replays other known-system-sender prefix transactions. | Family system-call handlers. |
| Backend | `refresh_fork_storage` cannot override a slot the running transaction already loaded. | The refreshed value is set as a journaled write, so its original value (for `SSTORE` gas and refunds) stays the pre-refresh value. | evm2 slot override, like `AccountHandle::override_balance`. |
| Backend | The local database is evm2's `EmptyDB`, which reports unknown accounts as absent. | REVM's `EmptyDBWrapper` reported them as empty existing accounts; only pre-Spurious Dragon execution can observe the difference. | None needed. |
| Anvil storage | Resolved: Anvil owns evm2-typed `MemDb`, `CacheDB`, `StateSnapshot` and dump/load types; serialized dumps keep the pre-migration layout. | `foundry-fork-db` is still read through Anvil's REVM-free boundary types. | None needed. |
| Anvil Celo | Anvil executes Celo CIP-64 transactions through their inner EIP-1559 body. | The fee currency is ignored, so gas is charged in CELO instead of the fee currency. | evm2 Celo transaction type. |
| Anvil Base | base-evm2 has no EIP-8130 handler. | EIP-8130 pool admission, calls, gas estimation and simulation fail with `Eip8130TransactionRejected`; the pool payer reservation charges `gas_limit * max_fee_per_gas` without the payer-authorization cost. Nonce reads (`base_getNonce`) still work. | base-evm2 EIP-8130 handler. |
| Anvil Base | `base-common-evm` (REVM) owned the Canyon create2 deployer, the Denim `BaseTime` predeploy and the Zenith system-account stub. | Anvil writes them directly into its database at genesis, reset and load with bundled bytecode artifacts (`crates/anvil/src/eth/backend/mem/bytecode/`); behavior matches, but the artifacts must be bumped by hand with the Base pin. | Engine-neutral Base genesis helpers. |
| Anvil OP | op-evm2 has no post-execution (`0x7D`) transactions. | Calls, estimates and pool admission of post-exec transactions fail explicitly. | op-evm2 PostExec handler. |
| Anvil simulate | `eth_simulateV1` builds its block access list from per-transaction pending state. | Writes of block-level system calls (EIP-2935/4788/7002/7251) are missing from the simulated BAL hash. | Record system calls into `Bal`. |
| Anvil simulate | Precompile moves (`movePrecompileToAddress`) are implemented by Anvil's precompile wrapper only for the Ethereum family. | Other families reject moves with an explicit error. | Move support in the family precompile providers. |
| Anvil precompiles | `PrecompileFactory` returns a fixed `(Address, PrecompileId, DynPrecompile)` list; REVM's dynamic precompile lookup (`install`) has no equivalent. | Library users must list precompile addresses up front. | evm2 lookup-based precompile provider. |
| Anvil Tempo | Tempo's evm2 integration has no standalone pool-validation entry point. | Pool admission runs the transaction against the latest state with `skip_valid_after_check` and `skip_liquidity_check`, then discards it; fee-token shortfalls surface as the generic insufficient-funds error instead of `insufficient fee token balance`. | Tempo evm2 pool validator. |
| Anvil tests | Remaining integration failures that need follow-up. | `optimism::test_tempo_fields_do_not_override_op_deposit_classification` (`OutOfGas`), `tempo::test_tempo_eip7702_replay_signature_override` (authority override not applied to Tempo 7702 replays) and `tempo::test_gas_estimation_with_value_fails` (error now `-32003 value transfer not allowed`). | Per-test follow-up, preserving the existing assertions. |
| Dependencies | `foundry-evm-hardforks`, `foundry-evm-networks`, `foundry-common-fmt` and the native fork cache depend on no REVM crate, but external crates still compile REVM (`revm`, `alloy-evm`) transitively: `tempo-evm` (evm2 integration) through `reth-evm`, `reth-evm-ethereum` and `reth-consensus`; `base-consensus-upgrades`, `base-protocol`, `base-common-chains`, `base-common-network`, `base-common-rpc-types` and `base-common-precompiles` (`foundry-evm-traces`); `base-common-consensus` with its `evm` feature, which `foundry-evm-core` enables, so `base-evm2` and `base-common-eip8130` pull REVM in unified builds. Foundry retains REVM's serialized fork block metadata type. | REVM is compiled but not executed. | REVM-free releases of these crates, or dropping the `evm` features that enable it. |

## Dependencies

All migration dependencies use the `mablr` forks. Foundry and its satellites share
[`mablr/evm2@mablr/foundry-completion`](https://github.com/mablr/evm2/tree/mablr/foundry-completion).
It adds suspended-state lifecycle APIs (`finalize_transaction`, warmth restoration,
`cool_loaded_state`, `set_prewarm_set`) and independent `BLOB_BASE_FEE_CHECK` policy on upstream
`1dc7f6a`, including account warmth control from evm2#705, plus resolution of already-present
delegation code before Prague, matching REVM.
Authorization processing remains Prague-gated. The lockfile must contain one evm2 graph, not
separate engines for different networks.

[`foundry-core@mablr/fork-db-evm2-completion`](https://github.com/mablr/foundry-core/tree/mablr/fork-db-evm2-completion)
stores native evm2 accounts and bytecode. Existing JSON/zstd caches remain readable, including
legacy analyzed bytecode and EIP-7702 delegation. RPC cache reads remain separate from execution
commits. [`evm2-networks`](https://github.com/mablr/evm2-networks/tree/main) provides the network
adapters. Tempo's integration uses
[`mablr/reth@mablr/evm2-migration`](https://github.com/mablr/reth/tree/mablr/evm2-migration).

`tempo-evm` and `tempo-hardfork` use
[`mablr/tempo@mablr/evm2-migration`](https://github.com/mablr/tempo/tree/mablr/evm2-migration).
`tempo-primitives-evm2` and `tempo-precompiles-evm2` alias that revision while RPC types retain
the REVM-era Tempo pin on the same fork.

## Completion follow-ups

Dependency readiness is not full Foundry parity. Do not bless failing snapshots or weaken
validation to hide these differences. The existing tests remain executable reproducers.

- `isolated_snapshot_enclosing_revert`, `StateSnapshots.t.sol` and `SnapshotCapsulePolicy.t.sol`:
  reverting a snapshot replaces the state/journal, so a later ancestor revert cannot undo just
  the replacement's effects. A follow-up needs journaled restore semantics, preserving fork
  selection and source writes across switches while rolling back applicable target writes.
- `Issue6032.t.sol` and `Fork2.t.sol`: distinguish transaction-loaded state, per-context cached
  observations and accepted writes from the shared RPC cache. Fuzz and symbolic setup discovery
  already uses the per-context cache, which includes committed-session reads. Do not enumerate
  the shared RPC cache: it can contain persisted or sibling-execution reads. Remaining fork-switch
  visibility and touched-only rediscovery need separate lifecycle semantics.
- Base `registry::tests::eip8130_is_rejected_as_unsupported` in evm2-networks and Foundry's
  `base_eip8130_*`: implement the complete phase and fee contract together. The TxContext getter
  has a smaller prerequisite: `PrecompileProvider::execute` has no active transaction origin.
  On Cobalt, calling `0x813000000000000000000000000000000000aa02` with `0xbead3f50` returns empty
  bytes instead of the sender ABI word. Expose read-only transaction context; do not cache origin
  in a network extension, which would become stale across raw/system message execution.
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
