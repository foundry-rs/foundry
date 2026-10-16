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
| Lifecycle | evm2 replaces its spec and `Version` (which carries the chain ID) only between transactions. | A fork switch or snapshot revert inside a transaction applies state and block at once and the spec or chain ID at the next transaction (`EnvTransition::Deferred`); `vm.setEvmVersion` also applies from the next transaction, such as from `setUp` to the test. | `Evm::set_execution_config` during execution. |
| Lifecycle | A fork switch accepts the running transaction's writes. | Like the REVM backend, an enclosing revert does not undo writes made before a fork switch. | None needed. |
| Transactions | evm2 frames borrow the transaction environment (origin, gas price, chain ID, blob hashes) for the whole transaction. | `vm.prank` with origin, `vm.txGasPrice`, `vm.blobhashes` and `vm.chainId` inside a transaction override the `ORIGIN`, `GASPRICE`, `BLOBHASH` and `CHAINID` opcodes at once; fee accounting keeps the transaction's values, and the chain ID applies to validation from the next transaction (`EnvTransition::Deferred`). | Mutable `TxEnv` for nested frames. |
| Cheatcodes | `vm.transact` and `vm.broadcastRawTransaction` execute through `Backend::transact*`, which cannot re-enter the cheatcode inspector that drives them. | The replayed transaction runs with the stack's tracing, logging and other non-cheatcode inspectors; REVM also ran the cheatcode inspector inside it. | Re-entrant inspector access from `Backend::transact*`. |
| Cheatcodes | evm2's `BlockEnv` has no excess blob gas. | `vm.blobBaseFee` sets the blob base fee from `fake_exponential`; `vm.getBlobBaseFee` returns the last value set, which snapshots and fork switches do not revert. | Excess blob gas on `BlockEnv`. |
| Cheatcodes | evm2 has no API to mark a warm account cold. | `vm.cool` cools the account's loaded storage slots but not the account. | `State::set_account_cold`. |
| Cheatcodes | evm2 does not expose accounts and slots that were only loaded. | `vm.dumpState` sees the accounts the running transaction changed or warmed and the accounts it created; `vm.cloneAccount`, `vm.copyStorage` and dumped storage see the accepted overlay plus the running transaction's journal. Accounts or slots that were only read without warming are omitted, and unlike REVM's journal the dump does not keep accounts that a fork switch, fork roll or enclosing revert removed from the active fork's state. | Loaded-state iteration on `State`. |
| Cheatcodes | evm2 has no re-entrant host call from a storage hook. | Storage hook callbacks run on a separate frame EVM with warmth restored afterwards. | None needed. |
| Cheatcodes | A snapshot revert or fork switch inside an isolated call replaces state the parent frame cannot merge. | The parent adopts the isolated child's whole state. | None needed. |
| Cheatcodes | Monad's context snapshots and the isolated-snapshot journal tracking relied on REVM journals. | Snapshots use `Backend::snapshot_state`/`revert_state`; logs from `vm.executeTransaction` are not merged into the receipt (as before). | None needed. |
| Isolation | evm2 has no per-call transaction boundary inside a running transaction. | With `isolate`, each top-level call runs as a nested fee-less transaction from the call's caller and nonce with the call's gas plus intrinsic gas, raised to the EIP-7623 floor and capped at the block gas limit; the parent frame is charged its execution and state gas and gets its refund on success. EIP-8037 state gas the parent precharged is not credited, cheatcode precompile flags do not apply, and without cheatcodes `ORIGIN` is the call's caller. | Nested transactions in the evm2 host. |
| Isolation | evm2 uses a frame's message depth both for the call-depth limit and as the depth inspectors and cheatcodes see. | An isolated call keeps its depth in the surrounding transaction, so it can nest 1023 calls below a top-level call; REVM restarted the depth limit at the nested transaction and allowed 1024. | Separate call-depth limit base in the evm2 host. |
| Traces | evm2 fires `step_end` for call and create opcodes after the child frame returns, and steps code-less accounts through a single `STOP`. | Step traces (`forge test --json` at high verbosity, the debugger) report the gas a `CALL*`/`CREATE*` step consumed, child included, instead of the gas remaining after forwarding (REVM, geth struct logs), and show one `STOP` step in calls to accounts without code. | None needed. |
| Executor | `call_tx`, `transact_tx` and block replay take converted network transactions, whose request fields are gone. | Their results report a zero `stipend`; `calculate_stipend` on a request omits EIP-7702 authorization costs. | None needed. |
| Tempo | Tempo's EVM extension shares transaction-local state (storage action buffers, non-creditable slots) with its precompiles through `Rc`, which cannot cross threads. | Parked contexts and snapshots store a detached copy (`StoredExt`) that keeps fee settings and resets that transaction-local state. | Thread-safe Tempo extension state. |
| OP | op-evm2 has no post-execution (`0x7D`) transactions. | Converting one fails with an explicit error. | op-evm2 PostExec handler. |
| Base | base-evm2 registers no EIP-8130 handler; stateful Beryl/Cobalt precompiles are missing. | Executing an EIP-8130 transaction fails as an unsupported transaction type. | base-evm2 handlers. |
| Tempo | Foundry's RPC types and Tempo's evm2 integration pin different Tempo revisions. | Envelopes cross over as their exact EIP-2718 encoding. | Single Tempo pin. |
| Fork cache | RPC endpoints report absent accounts as empty, so `ForkDb` cannot tell an existing empty account from an absent one. | `ForkDb` follows the EIP-161 rule of the latest EVM built or environment installed over the fork: with EIP-161 empty remote accounts load as absent, before it as existing (as REVM loaded them), so pre-Spurious Dragon execution sees absent empty accounts as existing. | None needed. |
| Fork cache | `foundry-fork-db` stores REVM-typed accounts. | `ForkDb` converts at the read boundary. | evm2-typed fork cache in `foundry-core`. |
| Fuzz | The evm2 accepted overlay holds only committed changes, while REVM's fork `CacheDB` also cached every account `setUp` read. | `EvmFuzzState::new` seeds the dictionary from accounts, code and storage `setUp` accepted; accounts it only read from a fork are not seeded. | Seed from the fork cache as well. |
| Fuzz | evm2 reports account changes, not REVM's touched flag. | Contracts are discovered from accounts a call changed; an account it only called without changing is not rediscovered. | None needed. |
| Symbolic | The evm2 accepted overlay holds only committed changes, while REVM's fork `CacheDB` also cached every account `setUp` read. | Symbolic calls to a symbolic address consider accounts that `setUp` accepted and accounts the symbolic run itself loaded; contracts `setUp` only read from a fork are not candidate targets. | Enumerate the fork cache as well. |
| Replay | Celo CIP-64 fee-currency transactions have no evm2 transaction type. | Replaying or `vm.transact`-ing one fails with an explicit error. | evm2 Celo transaction type. |
| Replay | The Celo transfer precompile and the BSC P256 precompile are not installed. | All execution, including local Forge, Script and Chisel runs with the Celo network, fork prefix replay and `vm.transact`, uses the network family's standard precompiles; calls to them execute as calls to empty accounts and diverge from chain. | Network precompile injection in `FoundryEvmFactory::precompiles`. |
| Replay | Protocol system transactions (known system senders, except `0x7E` deposits) have no native representation; Monad protocol system calls included. | They are skipped when replaying a block prefix, so state they write is missing. | Family system-call handlers. |
| Monad | Synthetic transactions on a fork (`transact_from_tx`, calls) do not reconstruct the block-position context. | They prepare Monad's reserve-balance context with `FoundryEvmExt::for_transaction`; `vm.transact` and prefix replay use the exact block context. | Track the fork's block position in `ForkInfo`. |
| Lifecycle | evm2's runtime version uses `u64::MAX` both for "no EIP-7825 cap" before Osaka and for Foundry's disabled cap. | A disabled cap is kept across spec changes, so with `enable_tx_gas_limit` on a network whose default spec predates the cap (OP-stack, Base), resolving to a later spec keeps the cap disabled. | A separate tx gas cap policy flag on `Version`. |
| Script | evm2 transactions are opaque, so a converted signed transaction cannot have its fees zeroed. | Fixed-gas simulation of a signed transaction executes the request rebuilt from its envelope (same type, calls, access and authorization lists) with zero fees, priced as a simulation: the OP-stack L1 data fee is not charged. Broadcast and `--resume` still send and decode exactly the persisted signed bytes. | None needed. |
| Verify | `verify-bytecode` replaces the creation transaction's input with the local creation code. | The target executes as the request rebuilt from its on-chain envelope with the new input, priced as a simulation; prefix transactions execute as signed. On Monad, protocol system transactions in the prefix are skipped like other replays. | None needed. |
| Cast run | Monad protocol system calls have no evm2 replay path, and block replay always skips prefix transactions from known system senders. | `cast run` of a Monad system transaction fails explicitly and points to `--debug-trace-transaction`, even with `--replay-system-txes`; that flag still admits other system transaction targets and `0x7E` prefix transactions, but no longer replays known-system-sender prefix transactions. | Family system-call handlers. |
| Backend | `refresh_fork_storage` cannot override a slot the running transaction already loaded. | The refreshed value is set as a journaled write, so its original value (for `SSTORE` gas and refunds) stays the pre-refresh value. | evm2 slot override, like `AccountHandle::override_balance`. |
| Backend | The local database is evm2's `EmptyDB`, which reports unknown accounts as absent. | REVM's `EmptyDBWrapper` reported them as empty existing accounts; only pre-Spurious Dragon execution can observe the difference. | None needed. |
| Anvil storage | Resolved: Anvil owns evm2-typed `MemDb`, `CacheDB`, `StateSnapshot` and dump/load types; serialized dumps keep the pre-migration layout. | `foundry-fork-db` is still read through Anvil's REVM-free boundary types. | None needed. |
| Anvil tracing | evm2-inspectors' JS tracer needs `boa_engine` 0.22, which conflicts with the workspace pin. | `debug_trace*` with a JavaScript tracer fails with an explicit `invalid params` error, even with the `js-tracer` feature. | Align the `boa_engine` pin. |
| Anvil Celo | Anvil executes Celo CIP-64 transactions through their inner EIP-1559 body. | The fee currency is ignored, so gas is charged in CELO instead of the fee currency. | evm2 Celo transaction type. |
| Anvil Base | base-evm2 has no EIP-8130 handler. | EIP-8130 pool admission, calls, gas estimation and simulation fail with `Eip8130TransactionRejected`; the pool payer reservation charges `gas_limit * max_fee_per_gas` without the payer-authorization cost. Nonce reads (`base_getNonce`) still work. | base-evm2 EIP-8130 handler. |
| Anvil Base | `base-common-evm` (REVM) owned the Canyon create2 deployer, the Denim `BaseTime` predeploy and the Zenith system-account stub. | Anvil writes them directly into its database at genesis, reset and load with bundled bytecode artifacts (`crates/anvil/src/eth/backend/mem/bytecode/`); behavior matches, but the artifacts must be bumped by hand with the Base pin. | Engine-neutral Base genesis helpers. |
| Anvil OP | op-evm2 has no post-execution (`0x7D`) transactions. | Calls, estimates and pool admission of post-exec transactions fail explicitly. | op-evm2 PostExec handler. |
| Anvil Monad | Protocol system transactions (sender `SYSTEM_ADDRESS`) have no evm2 replay path. | Replaying one (transaction-hash fork replay, forced replay mining) fails with an explicit `Rejected` error; `anvil_reorg` system calls are mined as ordinary impersonated calls to the staking contract instead of the dedicated syscall. | Monad system-call handler in monad-evm2. |
| Anvil simulate | `eth_simulateV1` builds its block access list from per-transaction pending state. | Writes of block-level system calls (EIP-2935/4788/7002/7251) are missing from the simulated BAL hash. | Record system calls into `Bal`. |
| Anvil simulate | Precompile moves (`movePrecompileToAddress`) are implemented by Anvil's precompile wrapper only for the Ethereum family. | Other families reject moves with an explicit error. | Move support in the family precompile providers. |
| Anvil precompiles | `PrecompileFactory` returns a fixed `(Address, PrecompileId, DynPrecompile)` list; REVM's dynamic precompile lookup (`install`) has no equivalent. | Library users must list precompile addresses up front. | evm2 lookup-based precompile provider. |
| Anvil precompiles | BSC's P256 precompile is added from Haber on, but an Osaka-or-later Ethereum spec on a pre-Haber BSC fork still exposes Ethereum's P256 precompile. | Pre-Haber BSC forks at Osaka see a precompile the chain did not have. | Per-chain precompile removal. |
| Anvil Tempo | Tempo's evm2 integration has no standalone pool-validation entry point. | Pool admission runs the transaction against the latest state with `skip_valid_after_check` and `skip_liquidity_check`, then discards it; fee-token shortfalls surface as the generic insufficient-funds error instead of `insufficient fee token balance`. | Tempo evm2 pool validator. |
| Anvil blobs | evm2 checks `max_fee_per_blob_gas` against the block blob base fee even when the base fee check is disabled; REVM skipped it. | `eth_simulateV1` calls without a blob fee cap fail with `blob_gas_price is greater than max_fee_per_blob_gas` (6 `eip4844::simulate_v1_*` tests); `can_send_eip4844_transaction_eth_send_transaction` also fails with `OpcodeNotFound`, not yet diagnosed. | Feature-gate the blob fee check in evm2. |
| Anvil tests | Remaining non-fork integration failures that need follow-up. | `anvil_api::reverting_prague_transition_rolls_back_mining`, `gas::test_estimation_with_print_traces`, `monad::monad_mining_tracks_eip7702_authorities`, `optimism::test_tempo_fields_do_not_override_op_deposit_classification` (OutOfGas), `tempo::test_tempo_eip7702_replay_signature_override` (recover override not applied to Tempo 7702 replays) and `tempo::test_gas_estimation_with_value_fails` (error now `-32003 value transfer not allowed`). Fork and RPC tests were not run. | Per-test follow-up. |
| Anvil build | `foundry-cli` (Anvil's default `cli` feature) depends on `foundry-cheatcodes`, which does not compile on the branch yet. | `cargo check -p anvil --all-features` fails in `foundry-cheatcodes`; Anvil itself checks with `--no-default-features --features optimism,base,monad,js-tracer,asm-keccak`. | Cheatcodes layer. |
| Dependencies | `foundry-evm-hardforks`, `foundry-evm-networks` and `foundry-common-fmt` depend on no REVM crate, but external crates still compile REVM (`revm`, `alloy-evm`) transitively: `tempo-evm` (evm2 integration) through `reth-evm`, `reth-evm-ethereum` and `reth-consensus`; `base-consensus-upgrades`, `base-protocol`, `base-common-chains`, `base-common-network`, `base-common-rpc-types` and `base-common-precompiles` (`foundry-evm-traces`); `base-common-consensus` with its `evm` feature, which `foundry-evm-core` enables, so `base-evm2` and `base-common-eip8130` pull REVM in unified builds; and `foundry-fork-db`. | REVM is compiled but not executed. | REVM-free releases of these crates, or dropping the `evm` features that enable it. |

## Dependencies

evm2 is patched workspace-wide onto `mablr/evm2@mablr/foundry-lifecycle`, which adds the
suspended-state lifecycle API (`finalize_transaction`, warmth restoration, `set_prewarm_set`) on
top of the revision Tempo and `evm2-networks` share. Keep the three in lockstep.

`tempo-evm` and `tempo-hardfork` name Tempo's evm2 revision. `tempo-primitives-evm2` and
`tempo-precompiles-evm2` alias it while the REVM-era `tempo-primitives` (used by the RPC types)
and `tempo-precompiles` pins remain in use.
