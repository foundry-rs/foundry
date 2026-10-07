# What reth-anvil needs from reth

reth-anvil keeps only the dev-node behaviour: impersonation, mining modes, time control, state
writes, snapshots, forking, and the `anvil_*` namespace. Everything else is reth. Where reth has no
hook for a piece of that behaviour, this crate carries a copy or a workaround. This file lists each
one, the reth change that would replace it, and what it costs here, so the upstream work has a
ready list and the crate shrinks as hooks land.

Size today: `crates/anvil` is about 84k lines of Rust; `crates/reth-anvil` is about 13k plus
8.9k of tests, with about 4k of the 13k in the items below. The target is to delete
`crates/anvil` and end up net negative.

## Workarounds and the hooks that remove them

| What reth-anvil does | Where | Lines | Reth hook that replaces it |
| --- | --- | ---: | --- |
| Copies `EngineNodeLauncher::launch_node` to swap the provider type passed to the engine and RPC | `src/launcher.rs` | ~460 | A `with_blockchain_db` style hook on the launcher, or a launcher generic over the `FullProvider` the node builder picks |
| Copies remote reads into the database before `newPayload`, because the engine validates against `OverlayStateProviderFactory`, which reads MDBX tables directly and bypasses `StateProviderFactory` | `src/provider.rs` (`materialize_fork_reads`), `src/fork.rs` (`RemoteReads`) | ~120 | Let the engine validator take the state provider from the node's `StateProviderFactory`, or expose a hook to wrap the provider it executes against |
| Rewinds through reth's `allow_unwind_canonical_header` engine option: a forkchoice update onto a canonical ancestor unwinds the in-memory chain, and the persistence task removes the blocks above it. Around it, the miner waits for the database to catch up, a stand-in pending block gives genesis the parent state the unwind loads, and the provider emits the reorg notification the unwind path does not | `src/miner.rs` (`rewind`), `src/provider.rs` (`RewindHooks`) | ~110 | An unwind that completes with the persistence removal, works for genesis, and notifies subscribers |
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
| Rejects every transaction past `--max-transactions` in the executor wrapper, so the payload builder leaves it in the pool for the next block | `src/evm.rs` (`AnvilBlockExecutor`) | ~25 | A transaction count limit in `PayloadBuilderArgs` |
| Replaces `eth_gasPrice` to return the base fee alone under `--disable-min-priority-fee` | `src/api.rs` (`EthExtApi`) | ~10 | A gas price oracle option for a zero tip |
| Installs a precompile at Hardhat's console address that decodes `console.log` calls, and prints the lines of every mined transaction from the executor wrapper, because the payload builder has no inspector hook. The precompile address is warm at the start of a transaction, so the first `console.log` of a transaction costs 2,500 gas less than on anvil | `src/console.rs`, `src/evm.rs` | ~120 | An inspector hook on the payload builder, or an `Inspector` slot on `ConfigureEvm::evm_for_block` |
| Replaces `eth_estimateGas`: probes the calls between reth's estimate and 1.5% below it to return the exact limit, as anvil does, and funds the zero address when a request without `from` carries fee fields, because reth caps the estimate by that balance | `src/api.rs` (`EthExtApi`) | ~50 | A configurable `ESTIMATE_GAS_ERROR_RATIO`, and no allowance cap for a request without `from` |
| Reports a failed block build as `failed to build payload <id>: missing payload`, because the payload service logs the build error and resolves with `MissingPayload`; anvil reports the EVM error | `src/miner.rs` | ~5 | Keep the job's error and return it from `resolve` |
| Replaces reth's engine validator to accept payload attributes with a timestamp at or below the parent's, and to convert a payload of a block before London to a header without a base fee, which the engine API cannot express | `src/engine.rs` | ~100 | A dev-mode switch for the timestamp check, and an engine API that carries an optional base fee |
| Replaces `eth_call` and `eth_estimateGas` to drop fee fields below the base fee, `eth_callMany` to answer every bundle and move each one a block past the one before, `eth_baseFee` to report the next-block override, `eth_sendRawTransactionSync` and `eth_sendTransactionSync` to report a timeout with code 4 and the hash, `eth_sendTransaction` to fall back to the largest gas limit when the estimate reverts, and `web3_clientVersion` to name this node | `src/api.rs` (`EthExtApi`, `Web3ExtApi`) | ~150 | Anvil's semantics for these are dev-node conveniences; a dev mode in reth could carry them |
| Serializes `eth_sendTransaction` and `eth_sendRawTransaction` per sender, from the nonce selection to the pool insertion, so concurrent requests without a nonce get distinct nonces: reth reads the next nonce and inserts without a lock | `src/api.rs` (`send`, `send_raw`) | ~30 | A per-sender lock around `send_transaction_request`, or a nonce reservation in the pool |
| Rejects a replacement whose fee does not exceed the pooled transaction's before it reaches the pool, because `PriceBumpConfig` with a zero bump replaces at an equal fee, and anvil requires a higher one; reth's default ten percent bump rejects anvil's `gas_price + 1` replacements | `src/api.rs` (`ensure_replacement_priced`), `src/pool.rs` | ~40 | A strict-inequality option on `PriceBumpConfig`, or a bump below one percent |
| Replaces the pending block environment builder so calls at `pending` see the next block's timestamp, coinbase, and prevrandao as the miner sets them; reth's `BuildPendingEnv` uses the parent timestamp plus twelve seconds | `src/pending.rs` | ~110 | A `PendingEnvBuilder` hook on `EthereumEthApiBuilder` |
| Chains automine blocks after a block that leaves ready transactions behind, waiting for the pool to see each block first, and groups transactions that arrive within five milliseconds into one block, as anvil's instant miner does; a block without transactions idles automine until the pool changes | `src/mining.rs`, `src/miner.rs` (`follow_up`, `MineIfPending`) | ~90 | A local miner mode that drains the pool |
| Gives the first block the genesis base fee through a one-shot override, where reth applies the EIP-1559 decrease of an empty parent; replaces `eth_feeHistory` to take the entry after the newest block from that block or from the next-block override, and to report a zero gas-used ratio for a block without a gas limit instead of NaN | `src/node.rs`, `src/api.rs` (`eth_fee_history`) | ~50 | A fee history that reads the child block; an initial base fee option for dev chains |
| Rejects at `eth_sendTransaction` and `eth_sendRawTransaction` a fee cap below the next block's base fee, as anvil does; reth's pool parks the transaction until the base fee drops. Fails a priced `eth_call` whose sender cannot pay for its gas and value, as anvil does; reth runs calls without the balance check | `src/api.rs` (`ensure_fee_cap`, `ensure_call_funds`) | ~50 | Pool and call options for these checks |
| Installs the `ArbSys` precompile on Arbitrum chains, through precompile builders that get the block number | `src/evm.rs` (`PrecompileBuilder`), `src/network/ethereum.rs` | ~15 | A block-aware precompile hook on `EvmFactory` |
| Gives impersonated transactions a signature with the sender in `r`, so the transactions of different impersonated senders get different hashes, as anvil's impersonated hash does | `src/impersonation.rs` | ~5 | A sender-attributed transaction that reth hashes with its sender |
| Replaces `eth_newFilter` to drain the block the filter is installed on, so the filter reports the blocks after it, as anvil's does; reth's first poll includes the install block. Gives a revert without data the empty `data` anvil reports | `src/api.rs` (`eth_new_filter`, `with_revert_data`), `src/node.rs` | ~30 | Install filters at the next block; `data: "0x"` on empty reverts |
| Replaces `eth_newBlockFilter` like `eth_newFilter`, `eth_getUncleCountByBlockHash` and `ByBlockNumber` to fail for an unknown block, `eth_signTransaction` to fill the chain id and the gas limit, `eth_signTypedData_v4` as an alias of `eth_signTypedData`, and `eth_getTransactionCount` at `pending` to answer from the pool and the latest state, because reth builds and caches its pending block for the lookup, and the cached block then misses the transactions of the next second | `src/api.rs` (`EthExtApi`) | ~90 | A pending nonce that does not build a block; `eth_signTypedData_v4` |
| Retries `eth_getTransactionReceipt` and `eth_getTransactionByHash` with the transaction's block in the RPC cache when the lookup failed to recover the sender from the signature: an impersonated transaction has no valid signature, and reth's disk path recovers instead of reading the senders table | `src/api.rs` (`cache_block_of`) | ~30 | Read `TransactionSenders` in the RPC lookups |
| `anvil_dropTransaction` removes the sender's later transactions, `anvil_setNonce` and `anvil_setBalance` tell the pool the new values, and a snapshot revert brings the pool back to the snapshot: the transactions mined since return, the ones sent since go | `src/api.rs` (`anvil_revert`, `restore_pool`, `sync_pool_account`) | ~110 | Pool hooks for dropping dependents and for a pool snapshot |
| Wraps the database of every EVM to answer `BLOCKHASH` for the blocks below the fork block from the fork, because the engine executes against the local database, whose static files start at the fork block, and `StateProviderDatabase` reads a missing hash as zero | `src/evm.rs` (`ForkHashDb`, `AnvilEvm`) | ~110 | A block hash hook on the engine's state provider, or `StateProviderDatabase` falling back to a configurable source |

## Gaps that are not hooks

These are not yet implemented here and do not need a reth change to be implemented, but would also
be free if reth had a dev mode:

- `anvil_setChainId` relaunches the node from a state dump, so the state and the height survive but
  earlier blocks are no longer served; anvil keeps them.
- `--disable-block-gas-limit` sets the block gas limit to `u64::MAX` instead of only skipping the
  check, because reth's payload builder and pool enforce the header's limit.
- `--disable-min-priority-fee` leaves `eth_maxPriorityFeePerGas` and `eth_feeHistory` to reth's
  gas price oracle; only `eth_gasPrice` drops the tip, as in anvil.
- `--prune-history`, `--max-persisted-states`, and `--transaction-block-keeper` are accepted and have
  no effect: reth keeps the full history on disk, which is what these flags bound in anvil's
  memory.
- Reth caches its pending block for a second, so a transaction that reaches the pool shows in
  `eth_getBlockByNumber("pending")` and in calls at `pending` up to a second late; anvil builds
  the pending block on every request. Calls and estimates without a block run at `latest`, as on
  reth; anvil runs them on the pending block, so an estimate there sees the pool's transactions.
- `eth_getFilterChanges` does not report the logs a reorg or a snapshot revert removed with
  `removed: true`; anvil and geth do.
- A `genesis.json` header carries the forks the node runs and the root of the whole genesis
  state; anvil's header carries the JSON config's forks and the state root of the alloc alone, so
  the genesis hash differs.
- Revm clears the storage of an account without balance, nonce, or code at the end of a block,
  so `anvil_setStorageAt` on such an account does not stick; anvil keeps the storage.
- `eth_getProof` proves the state of the latest block; anvil's proofs include the state writes it
  has not mined yet.
- `eth_createAccessList` for a call without fee fields runs at a zero base fee on reth; anvil,
  like geth, runs it at the block's base fee.
- `anvil_reset` restarts from the configured genesis; anvil's reset carries the next-block base
  fee override into the new genesis header.
- Reth's pool validator rejects transaction types by the hardfork of the latest block when the
  pool is built, with reth's messages (`transaction type not supported`,
  `EIP-1559 transactions are disabled`); a gas limit above the block's is
  `exceeds block gas limit`, and one above the EIP-7825 cap is `gas limit too high`.
- `--print-traces` and `--steps-tracing` are accepted and have no effect: printing the trace of
  every mined transaction needs an inspector during block building, or a replay of every block.
- Networks: Optimism and Base through `op-reth` node types, which moved from the reth repository to
  `ethereum-optimism/optimism` and must be pinned to the same reth revision as this crate; Tempo
  through `tempo-node`. Monad runs
  (`src/network/monad.rs`) with its own `ConfigureEvm` on `monad-revm`; still missing are the
  protocol system envelopes anvil replays on reorgs and transaction-hash forks, the per-block
  hardfork profiles of a Monad fork, and signature overrides for EIP-7702 authorities.
- Monad reserve balances depend on the senders of the two ancestor blocks. Block execution gets them
  from the parent hash. An RPC call only carries a block number, so a call at the latest block runs
  on top of it, like anvil's pending block, and a call at an older block replays that block.

## Anvil's own tests

`tests/it/{anvil_api,api,transaction,gas,revert,logs,filter,pubsub,sign,txpool,genesis,proof,
block_index,storage_values}.rs` are anvil's modules of the same name with the in-process calls made async and the anvil-internal hooks removed
(`api.backend`, `api.execute`, pool types, `eth_callBundle`, the ready-transaction listener, the
state dump's transaction records, the fee manager's blob fee, the Optimism variants, the block
listener count). Ignored tests carry the reason on the attribute: the pending
call that expects the beacon root system call, the Arbitrum tip rule, the pending block cache
above, and the estimate that defaults to the pending block. Assertions on wall-clock seconds
became lower bounds, because blocks take longer to build here than on anvil, error messages
compare case-insensitively where reth's text differs only in case, and accept reth's text where
it differs. The other modules (`eip*.rs`, `otterscan.rs`, `traces.rs`, `fork.rs`, ...) are next.

## Differences that cast's tests show

Running `crates/cast/tests` against this node instead of anvil (the `anvil` dev-dependency renamed
to `reth-anvil`) passes everything that does not need Tempo, except these groups. None of them
is a missing method.

- Error text. Reth reports reverts as `execution reverted` with the revert data in the `data`
  field; anvil decodes custom errors into the message and prints `data: "0x"` for empty data.
  Tests that snapshot these messages need new snapshots.
- `eth_getBlockAccessListByBlockNumber` for a block without a list answers `block not found`
  (reth) instead of anvil's `block access list ... not found`.
- Storage on a plain address. `anvil_setStorageAt` on an account without balance, nonce, or code
  keeps the storage in anvil; here the next block clears the empty account (EIP-161). An account
  without nonce and code keeps the storage, but revm treats it as known to be empty once the
  account changes, so a code override in `eth_call` does not see it (`cast call --delegate` with a
  fresh sender).
- Reth serves an EIP-7928 block access list for every block it executed, also before Amsterdam;
  anvil only for blocks whose header carries one. `cast run` uses the list to skip replaying the
  earlier transactions of a block, so its progress output differs.

- Pending calls. Reth runs `eth_call` at `pending` on the latest state without the beacon root
  system call, so the EIP-4788 contract does not hold the pending block's root; anvil runs the
  call on the pending block.
- A tip above the fee cap. Reth's pool rejects it on every chain; anvil allows it on Arbitrum.
- Amsterdam state gas. Anvil does not charge EIP-8037 state gas (foundry-rs/foundry#17428);
  this node does, so a transaction on Amsterdam that creates state with a tight gas limit runs
  out of gas here. Tests that pin such gas limits need more gas.

Running `crates/forge/tests/cli` the same way passes everything that does not need Tempo, except
a `--fork-bal` test: anvil's block access lists carry no storage reads, reth's do, and forge's
parent cache then does not fall back to the endpoint for read-only slots.

The unit tests of `forge-script` and `foundry-evm-core` that spawn a node pass as well, except
those that need Tempo or Optimism.

## What Tempo needs

`tempo-node` hard-wires its EVM config, so reth-anvil cannot wrap it the way it wraps
`EthEvmConfig`. The sites, at tempo rev `6ef1f812`:

| Site | Problem |
| --- | --- |
| `crates/node/src/node.rs:555` | `TempoNode` sets `type EVM = TempoEvmConfig`. |
| `crates/node/src/node.rs:772` | `TempoPoolBuilder` only implements `PoolBuilder<Node, TempoEvmConfig>`. |
| `crates/node/src/node.rs:885` | `TempoPayloadBuilderBuilder` only implements `PayloadBuilderBuilder<Node, TempoTransactionPool<_>, TempoEvmConfig>`. |
| `crates/evm/src/assemble.rs:86` | `TempoBlockAssembler` only implements `BlockAssembler<TempoEvmConfig>`. |

`TempoTransactionValidator` (`crates/transaction-pool/src/validator.rs:94`) and
`TempoTransactionPool` (`tempo_pool.rs:59`) are already generic over `EvmConfig`, with
`TempoEvmConfig` as the default, so the pool side only needs the builders to pass the type through.
reth's `EthereumPoolBuilder` and `EthereumPayloadBuilder` show the shape: take the EVM config as a
type parameter bounded by `ConfigureEvm<Primitives = TempoPrimitives>` plus Tempo's own
`ConfigureTempoPoolEvm`, and let the assembler accept any config whose block executor factory is
Tempo's. The alternative is to copy the two builders and the assembler into this crate, about 1.5k
lines, and keep them in step with tempo.

## Things reth already fixes

Bugs open on anvil that reth-anvil does not have, because the logic is reth's:

- foundry-rs/foundry#17428: `eth_estimateGas` on Amsterdam returns 21,000 for a transfer that creates
  an account. Reth executes the transfer at 21,000 gas and only returns that estimate when the run
  succeeds. Test: `estimate_gas_charges_account_creation_state_gas_on_amsterdam`.
