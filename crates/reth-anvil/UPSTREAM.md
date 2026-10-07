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
| Replaces `eth_sendTransaction`, because reth rejects a request without `to`: alloy serializes a create recipient as `null`, which reads back as missing, and the dev signer then fails to build the transaction | `src/api.rs` (`EthExtApi`) | ~20 | Treat a missing `to` as a contract creation in `send_transaction_request` |
| Reports an unlimited balance for every valid transaction when `--disable-pool-balance-checks` is set, because the validator's `disable_balance_check` still reports the real balance and the pool parks a transaction the sender cannot afford yet | `src/pool.rs` (`AnvilValidator`) | ~10 | Let `disable_balance_check` also skip the pool's balance-based parking |
| Rewrites every EVM environment to set the code size limit, the memory limit, EIP-3607, the block gas limit check, and the EIP-7825 cap | `src/evm.rs` (`EvmSettings`) | ~40 | A `CfgEnv` overrides hook on `ConfigureEvm`, or dev-mode flags for these limits |
| Registers `eth_config` itself, because reth adds it only to its transport modules, not to the registry the in-process module is built from | `src/node.rs` | ~5 | Register `eth_config` in the RPC registry like the other `eth_*` methods |
| Runs its own RPC server in front of the node, forwarding every method to the current node's module, so `anvil_reset` to another fork and `anvil_setChainId` can relaunch the node without losing the endpoint, the connections, or the in-process API | `src/server.rs`, `src/node.rs` (`Relauncher`) | ~300 | A way to replace a running node's chain spec and database in place, or to restart the node behind reth's RPC servers |
| Replays the transactions before a fork transaction through the pool into the first local block, with the remote block's environment and the pool in arrival order | `src/node.rs` (`replay_fork_transactions`) | ~70 | A way to build and insert a block from a given transaction list |

## Gaps that are not hooks

These are not yet implemented here and do not need a reth change to be implemented, but would also
be free if reth had a dev mode:

- `anvil_setChainId` relaunches the node from a state dump, so the state and the height survive but
  earlier blocks are no longer served; anvil keeps them.
- `BLOCKHASH` of pre-fork blocks during block execution: the engine's state provider cannot reach the
  remote endpoint for block hashes.
- `--disable-block-gas-limit` sets the block gas limit to `u64::MAX` instead of only skipping the
  check, because reth's payload builder and pool enforce the header's limit.
- `--disable-min-priority-fee` only affects the pool: reth's gas price oracle suggests its own tip,
  so `eth_gasPrice` keeps a priority fee.
- `--max-transactions`: reth's payload builder has no cap on the number of transactions per block.
- Dev accounts keep their history on a forked chain: anvil resets their nonces and balances in the
  fork genesis, reth-anvil only sets the balances.
- Networks: Optimism and Base through `op-reth` node types, Tempo through `tempo-node`. Monad runs
  (`src/network/monad.rs`) with its own `ConfigureEvm` on `monad-revm`; still missing are the
  protocol system envelopes anvil replays on reorgs and transaction-hash forks, the per-block
  hardfork profiles of a Monad fork, and signature overrides for EIP-7702 authorities.
- Monad reserve balances depend on the senders of the two ancestor blocks. Block execution gets them
  from the parent hash. An RPC call only carries a block number, so a call at the latest block runs
  on top of it, like anvil's pending block, and a call at an older block replays that block.

## Differences that cast's tests show

Running `crates/cast/tests` against this node instead of anvil (the `anvil` dev-dependency renamed
to `reth-anvil`) passes everything that does not need Tempo, except these groups. None of them
is a missing method.

- Error text. Reth reports reverts as `execution reverted` with the revert data in the `data`
  field; anvil decodes custom errors into the message and prints `data: "0x"` for empty data.
  Tests that snapshot these messages need new snapshots.
- `eth_getBlockAccessListByBlockNumber` for a block without a list answers `block not found`
  (reth) instead of anvil's `block access list ... not found`.
- `eth_estimateGas` without `from` and with fee fields set fails with `gas required exceeds
  allowance (0)`: reth caps the estimate by the zero address's balance, anvil does not.
- Storage on a plain address. `anvil_setStorageAt` on an account without balance, nonce, or code
  keeps the storage in anvil; here the next block clears the empty account (EIP-161). An account
  without nonce and code keeps the storage, but revm treats it as known to be empty once the
  account changes, so a code override in `eth_call` does not see it (`cast call --delegate` with a
  fresh sender).
- `cast run` uses the block access lists reth serves to skip replaying the earlier transactions of
  a block, so its progress output differs, and the replay from that prestate reports less gas than
  the receipt for calls into contracts set with `anvil_setCode`; anvil serves no lists for its
  blocks. To be investigated with cast.

## What Tempo needs

`tempo-node` hard-wires its EVM config: `TempoPayloadBuilder`, `TempoPoolBuilder`, and
`TempoPayloadBuilderBuilder` name `TempoEvmConfig`, and `TempoBlockAssembler` only implements
`BlockAssembler<TempoEvmConfig>`. reth-anvil installs its anvil behaviour by wrapping a network's
`ConfigureEvm`, so Tempo cannot run here until those builders take the EVM config as a type
parameter, like reth's `EthereumPayloadBuilder` and `EthereumPoolBuilder` do. The alternative is to
copy the three builders and the assembler into this crate, about 1.5k lines.

## Things reth already fixes

Bugs open on anvil that reth-anvil does not have, because the logic is reth's:

- foundry-rs/foundry#17428: `eth_estimateGas` on Amsterdam returns 21,000 for a transfer that creates
  an account. Reth executes the transfer at 21,000 gas and only returns that estimate when the run
  succeeds. Test: `estimate_gas_charges_account_creation_state_gas_on_amsterdam`.
