# What reth-anvil needs from reth

reth-anvil keeps only the dev-node behaviour: impersonation, mining modes, time control, state
writes, snapshots, forking, and the `anvil_*` namespace. Everything else is reth. Where reth has no
hook for a piece of that behaviour, this crate carries a copy or a workaround. This file lists each
one, the reth change that would replace it, and what it costs here, so the upstream work has a
ready list and the crate shrinks as hooks land.

Size today: `crates/anvil` is about 84k lines of Rust; `crates/reth-anvil` is about 10k, with about
1.6k of that in the items below. The target is to delete `crates/anvil` and end up net negative.

## Workarounds and the hooks that remove them

| What reth-anvil does | Where | Lines | Reth hook that replaces it |
| --- | --- | ---: | --- |
| Copies `EngineNodeLauncher::launch_node` to swap the provider type passed to the engine and RPC | `src/launcher.rs` | ~460 | A `with_blockchain_db` style hook on the launcher, or a launcher generic over the `FullProvider` the node builder picks |
| Copies remote reads into the database before `newPayload`, because the engine validates against `OverlayStateProviderFactory`, which reads MDBX tables directly and bypasses `StateProviderFactory` | `src/provider.rs` (`materialize_fork_reads`), `src/fork.rs` (`RemoteReads`) | ~120 | Let the engine validator take the state provider from the node's `StateProviderFactory`, or expose a hook to wrap the provider it executes against |
| Rewinds the chain by editing `CanonicalInMemoryState` and the database itself, because an FCU below the in-memory chain is rejected and FCU to genesis fails with `StateForHashNotFound` | `src/provider.rs` (`rewind_to`), `src/miner.rs` | ~90 | An engine or tree API to unwind to any canonical ancestor and emit the reorg notification |
| Records impersonated senders in `SenderRecoveryCache` so RPC lookups find them | `src/evm.rs` (`tx_iterator_for_payload`) | ~15 | paradigmxyz/reth#27757 makes `transaction_by_hash` consult the cache; after that, only the recording stays |
| Wraps the block executor to apply `anvil_setBalance` and friends inside the next block, and overlays them on `latest`/`pending` reads | `src/evm.rs`, `src/state.rs`, `src/state_provider.rs` | ~450 | A pre-execution state hook on `ConfigureEvm` or `BlockExecutorFactory`, plus a provider-level overlay hook for pending state |
| Wraps the pool validator to accept impersonated and signature-overridden senders | `src/pool.rs` (`AnvilValidator`) | ~90 | A sender-attribution hook on `EthTransactionValidator`, or a validator that can be told to trust a recovered sender |
| Rewrites the EVM config to override gas limit, base fee, prevrandao, and beacon root per block | `src/evm.rs`, `src/block_env.rs`, `src/time.rs` | ~150 | `LocalPayloadAttributesBuilder` and `NextBlockEnvAttributes` accepting per-block overrides, so a dev node can set them without a config wrapper |
| Owns the miner: FCU with attributes, payload resolve, `newPayload`, FCU, with automine, interval, and manual modes | `src/miner.rs`, `src/mining.rs` | ~300 | A dev mining mode in reth's local miner (`--dev.block-time` exists; automine on pool events and manual `mine` do not) |
| Builds the `anvil_*`, `evm_*`, `hardhat_*`, and `personal_*` namespaces | `src/api.rs` | ~1000 | A `dev_*` namespace in reth with the same methods, which reth-anvil would alias to the anvil names |

## Gaps that are not hooks

These are not yet implemented here and do not need a reth change to be implemented, but would also
be free if reth had a dev mode:

- `anvil_reset` to a different fork endpoint or block, and `anvil_setChainId`: both change what the
  chain spec says and need a node relaunch. A reth dev node could rebuild its chain spec in place.
- Forking at a transaction hash, which replays the transactions before it in the fork block.
- `BLOCKHASH` of pre-fork blocks during block execution: the engine's state provider cannot reach the
  remote endpoint for block hashes.
- Networks: Optimism and Base through `op-reth` node types, Tempo through `tempo-node`, Monad through
  a `ConfigureEvm` built on `monad-revm`.

## Things reth already fixes

Bugs open on anvil that reth-anvil does not have, because the logic is reth's:

- foundry-rs/foundry#17428: `eth_estimateGas` on Amsterdam returns 21,000 for a transfer that creates
  an account. Reth executes the transfer at 21,000 gas and only returns that estimate when the run
  succeeds. Test: `estimate_gas_charges_account_creation_state_gas_on_amsterdam`.
