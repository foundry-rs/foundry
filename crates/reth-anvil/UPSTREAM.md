# What reth-anvil needs from reth

reth-anvil keeps only the dev-node behaviour: impersonation, mining modes, time control, state
writes, snapshots, forking, and the `anvil_*` namespace. Everything else is reth. Where reth has no
hook for a piece of that behaviour, this crate carries a copy or a workaround. This file lists each
one, the reth change that would replace it, and what it costs here, so the upstream work has a
ready list and the crate shrinks as hooks land.

Size today: `crates/anvil` is about 84k lines of Rust; `crates/reth-anvil` is about 17k plus
23k of tests, with about 6k of the 17k in the items below. The target is to delete
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
| Serves anvil's Beacon API routes (`/eth/v1/beacon/blobs/{block_id}`, `/eth/v1/beacon/genesis`) from a tower layer in front of the JSON-RPC server | `src/beacon.rs` | ~250 | Reth has no Beacon API; a blob route on the RPC server would do |
| Installs BSC's P256 verifier when Haber is active and overrides `ecrecover` for the signatures `anvil_impersonateSignature` registers, from the EVM factory | `src/evm.rs` (`install`, `cheat_ecrecover`) | ~60 | A chain-aware precompile hook on `EvmFactory` |
| Sends a blob transaction from a dev account by signing it through reth and attaching the sidecar as a pooled transaction, and fills `maxFeePerBlobGas`; reth's `eth_sendTransaction` signs without the sidecar and the pool rejects the result | `src/api.rs` (`send`) | ~30 | Keep the sidecar through `send_transaction_request` |
| Resolves `earliest` to the genesis block and anchors `safe` and `finalized` at it, as anvil does, for a chain whose genesis number is not zero | `src/provider.rs`, `src/miner.rs` | ~10 | `earliest_block_number` from the chain spec |
| Stops the RPC servers when the node handle drops while an in-process API keeps the node, as anvil does | `src/node.rs` (`Supervisor`) | ~20 | None; node behaviour |
| Routes the block access list methods: `null` before Amsterdam, an error above the head, and the fork endpoint for a fork's blocks and unknown hashes; gives `eth_simulateV1` calls the blob hashes of their sidecars | `src/api.rs` (`access_list_route`, `eth_simulate_v1`) | ~120 | BAL methods that honour the fork activation; sidecars in simulate calls |
| Wraps the database of every EVM to answer `BLOCKHASH` for the blocks below the fork block from the fork, because the engine executes against the local database, whose static files start at the fork block, and `StateProviderDatabase` reads a missing hash as zero | `src/evm.rs` (`ForkHashDb`, `AnvilEvm`) | ~110 | A block hash hook on the engine's state provider, or `StateProviderDatabase` falling back to a configurable source |
| Serves `eth_simulateV1` itself on reth's block builder, because reth's handler differs from anvil in the block sequence it fills in (anvil spaces blocks by the node's block interval, reth by a chain hint), the request gas budget (anvil caps a request at 50M gas), the error codes and messages, the transfer logs (reth journals them, so they reach the receipts and the bloom, and it logs `CALLCODE` and failed creates), the warm addresses after a precompile move (EIP-2929 warms the protocol addresses), the validation of precompile moves, and the state roots (off by default on reth); simulations on blocks before the fork block go to the fork endpoint | `src/simulate.rs` | ~600 | Hooks on `EthCall::simulate_v1` for the block interval, the gas budget, and the transfer log collector; `--rpc.compute-state-root-for-eth-simulate` on by default for dev nodes |
| Replaces `debug_traceTransaction` to report an unknown hash with code `-32001`, `trace_block`, `trace_replayBlockTransactions`, `trace_blockOpcodeGas`, `trace_filter`, and `debug_accountInfoAt` to answer from the fork endpoint for the fork block and the blocks before it, which the local chain does not store, `trace_transaction` to ask the fork for a hash the local chain does not know, `trace_get` to reject integer indices, `trace_rawTransaction` to reject a sender with code with anvil's message, and the pending block in block traces | `src/debug.rs` | ~300 | Fork-aware trace and debug handlers, or a block source hook in `TraceApi` and `DebugApi` |
| Accepts a transaction from a sender with code into the pool, because EIP-3607 is off for mining as it is for calls on anvil; reth's validator rejects it with `sender is not an EOA` | `src/pool.rs` (`AnvilValidator`) | ~15 | An EIP-3607 switch on `EthTransactionValidator` |
| Mines into the genesis coinbase unless `anvil_setCoinbase` set one, and takes back the pre-merge block reward reth's executor credits, because anvil pays none; `LocalPayloadAttributesBuilder` picks a random fee recipient per block | `src/time.rs` (`build_hooks`), `src/evm.rs` (`BlockReward`, `AnvilBlockExecutor::finish`) | ~40 | A fee recipient option on the local attributes builder; a block reward switch on the Ethereum executor |
| Dumps and loads anvil's state format with the chain's blocks, transactions, and historical states. A loaded dump's head becomes the genesis block, and the blocks below it are served through the fork backend in a dump mode, alone or on top of a fork endpoint, because reth's database starts at its genesis block. `anvil_loadState` at runtime merges the dump into the current chain and relaunches the node | `src/state_dump.rs`, `src/fork.rs` (`DumpHistory`, `from_dump`, `into_dump_fork`), `src/provider.rs` (`StateDump`), `src/config.rs` (`dump_chain_spec`) | ~900 | A way to import blocks with receipts and their states below the genesis block |
| Executes a JSON-RPC notification, a request without an id, and answers it with `204 No Content`, as anvil does; jsonrpsee acknowledges a notification without running its method | `src/logging.rs` (`NodeInfoService::notification`), `src/server.rs` (`NotificationLayer`) | ~70 | A switch on jsonrpsee's server to execute notifications |
| Serves the fork block's body, receipts, and body indices from the endpoint, because the fork block is the local genesis block and reth writes the genesis block with an empty body | `src/provider.rs` (`remote_for_body`, `remote_for_body_number`) | ~40 | A genesis block with a body, or a hook on the block readers |
| Recomputes the withdrawals and the parent beacon block root of the payload attributes for the block's final timestamp: `LocalPayloadAttributesBuilder` picks them for the wall-clock time, and the time manager may move the block across a hardfork, as the replayed block of a fork at a transaction hash does | `src/time.rs` (`build_hooks`) | ~30 | A timestamp source on `LocalPayloadAttributesBuilder` |
| Gives the pending block a zero parent beacon block root on a Cancun chain whose parent has none, the fork block of an older chain; reth takes the root's presence from the parent and builds no pending block | `src/pending.rs` | ~25 | The hardfork check in `BuildPendingEnv` |
| Leaves the console precompile out of `eth_config` | `src/node.rs` | ~15 | An `eth_config` hook for node-specific precompiles |

## Gaps that are not hooks

These are not yet implemented here and do not need a reth change to be implemented, but would also
be free if reth had a dev mode:

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
- Reth recovers EIP-7702 authorities from their signatures; anvil's signature overrides also
  apply to authorizations.
- Reth builds the pending block without anvil's check of the deposit contract's logs, so a
  malformed deposit log does not fail `eth_getBlockByNumber("pending")`.
- Anvil's precompile factory (`NodeConfig::with_precompile_factory`) is not served.
- Reth's pool requires the sidecar of a blob transaction; anvil mines a blob transaction sent
  without one. A blob call with a zero blob fee cap keeps the block's blob base fee in
  `eth_call` and `eth_simulateV1` on reth, which rejects the call when validation is off; anvil
  runs it at zero.
- `eth_simulateV1` shows the maximum nonce on a transaction that ran with it and no validation,
  as anvil does, but the transaction ran with nonce zero: revm cannot execute the maximum nonce.
- A trace replays the transaction with the precompiles of the node's current chain id, so after
  `anvil_setChainId` switches to a chain with other precompiles, the traces of the blocks mined
  before it change; anvil serves the traces it recorded at mining. Replays after a chain id
  change skip the EVM's chain id check, and the API checks the chain id of a request instead.
- Reth's `ots_getInternalOperations` reports no top-level create or transfer, and
  `ots_getBlockTransactions` pages a block's transactions in another order than anvil.
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

- Remote blocks keep only the transactions the EVM can execute: an Arbitrum system transaction,
  an OP-stack deposit, or another chain-specific type is left out of the blocks, receipts, and
  lookups the fork serves, and indices count the kept transactions. Anvil serves remote blocks
  as the endpoint returns them and skips those transactions only when it replays a block.
- A fork at a transaction hash replays the block under the source block's hardfork when it is
  older than the configured one; when it is newer, the replay runs under the configured one,
  because a reth chain spec activates hardforks once and for good.
- A call on a Cancun chain whose fork block lost its blob fields fails with reth's `excess blob
  gas missing` error instead of anvil's `Excess blob gas not set`, and such a chain has no
  pending block.
- Monad: protocol system transactions (the staking syscalls from the system address) are not
  accepted in `anvil_reorg`, because the pool and reth's payload builder would have to admit
  zero-gas, zero-price envelopes; foundry's `try_transact_monad_system_replay` already executes
  them. A Monad fork at a transaction hash keeps one hardfork schedule, where anvil records a
  hardfork profile per replayed block and restores it on rollback, and state dumps carry no
  Monad block participants or replay profiles. Anvil's Monad tests that read its pool or
  backend directly stay in `crates/anvil`.
- Arbitrum forks number blocks by the L2 block; anvil mirrors Arbitrum's L1 block numbers in
  `NUMBER` and in the blocks' `l1BlockNumber`.

## Anvil's own tests

`tests/it/{anvil_api,api,transaction,gas,revert,logs,filter,pubsub,sign,txpool,genesis,proof,
block_index,storage_values,eip2935,eip4844,eip6110,eip7702,eip7928,otterscan,beacon_api,ipc,wsapi,
anvil,traces,simulate,state,fork,fork_bal,fork_chains}.rs` are anvil's modules of the same name with the in-process calls made async and the anvil-internal hooks removed
(`api.backend`, `api.execute`, pool types, `eth_callBundle`, the ready-transaction listener, the
state dump's transaction records, the fee manager's blob fee, the Optimism variants, the block
listener count, the precompile factory, the JavaScript tracer). Ignored tests carry the reason on the attribute: the pending
call that expects the beacon root system call, the Arbitrum tip rule, the pending block cache
above, and the estimate that defaults to the pending block. Assertions on wall-clock seconds
became lower bounds, because blocks take longer to build here than on anvil, error messages
compare case-insensitively where reth's text differs only in case, and accept reth's text where
it differs. `state.rs`'s history pruning tests are ignored, because reth keeps the full history.
`fork_bal.rs` keeps the assertions on the state a fork serves and drops the ones on the fork
cache, because the node reads remote state lazily and has no block access list prefill; its
tests that need a deterministic block hash on a replacement endpoint start the endpoints from
the same genesis. `eth_config`'s blob schedule compares as JSON, because the in-process API
round-trips through EIP-7910's fields.

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

`TempoEvmConfig` has no factory seam (`TempoEvmConfig::new` builds `TempoEvmFactory::default()`), and
`TempoBlockExecutor` has no pre-execution callback, so the state writes, the transaction cap, and
the console cannot move into an EVM factory wrapper either. The plan without a tempo change: keep
Tempo's consensus, primitives, pool implementation, and eth API; give the node reth-anvil's
`AnvilEvmConfig<TempoEvmConfig>`; and build blocks with a dev-only sequential payload builder in
this crate (from `crates/payload/builder/src/lib.rs`, without prewarming and action replay) plus an
assembler input adapter around `TempoBlockAssembler`. A Tempo dev chain must also follow Tempo's
consensus: millisecond timestamps, DKG data in `extra_data` at epoch boundaries (T8 on), the T4
block layout without the subblock-metadata system transaction, TIP-20 fee tokens instead of native
balances, and host-side sender recovery that ignores a changed ECRECOVER. Tempo's eth API forces
`PendingBlockKind::None`, and its add-ons use `NoopEngineApiBuilder`.

## Things reth already fixes

Bugs open on anvil that reth-anvil does not have, because the logic is reth's:

- foundry-rs/foundry#17428: `eth_estimateGas` on Amsterdam returns 21,000 for a transfer that creates
  an account. Reth executes the transfer at 21,000 gas and only returns that estimate when the run
  succeeds. Test: `estimate_gas_charges_account_creation_state_gas_on_amsterdam`.
