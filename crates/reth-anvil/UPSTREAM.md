# What reth-anvil needs from reth

reth-anvil keeps only the dev-node behaviour: impersonation, mining modes, time control, state
writes, snapshots, forking, and the `anvil_*` namespace. Everything else is reth. Where reth has no
hook for a piece of that behaviour, this crate carries a copy or a workaround. This file lists each
one, the reth change that would replace it, and what it costs here, so the upstream work has a
ready list and the crate shrinks as hooks land.

Size: the old anvil crates (`crates/anvil`, about 84k lines of Rust) are gone; this crate is
about 21k lines plus 40k of tests, and the `anvil` binary builds from it.

## Workarounds and the hooks that remove them

| What reth-anvil does | Where | Lines | Reth hook that replaces it |
| --- | --- | ---: | --- |
| Copies `EngineNodeLauncher::launch_node` to swap the provider type passed to the engine and RPC | `src/launcher.rs` | ~460 | A `with_blockchain_db` style hook on the launcher, or a launcher generic over the `FullProvider` the node builder picks |
| Copies remote reads into the database before `newPayload`, because the engine validates against `OverlayStateProviderFactory`, which reads MDBX tables directly and bypasses `StateProviderFactory` | `src/provider.rs` (`materialize_fork_reads`), `src/fork.rs` (`RemoteReads`) | ~120 | Let the engine validator take the state provider from the node's `StateProviderFactory`, or expose a hook to wrap the provider it executes against |
| Rewinds through reth's `allow_unwind_canonical_header` engine option: a forkchoice update onto a canonical ancestor unwinds the in-memory chain, and the persistence task removes the blocks above it. Around it, the miner waits for the database to catch up, a stand-in pending block gives genesis the parent state the unwind loads, and the provider emits the reorg notification the unwind path does not | `src/miner.rs` (`rewind`), `src/provider.rs` (`RewindHooks`) | ~110 | An unwind that completes with the persistence removal, works for genesis, and notifies subscribers |
| Records impersonated senders in `SenderRecoveryCache` so RPC lookups find them | `src/evm.rs` (`tx_iterator_for_payload`) | ~15 | paradigmxyz/reth#27816 (reopening #27757) makes `transaction_by_hash` consult the cache; after that, only the recording stays |
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
| Replaces `eth_estimateGas`: probes the calls between reth's estimate and 1.5% below it, including limits below 21,000 after EIP-2780, rejects estimates above the request's gas allowance, funds the zero address when a request without `from` carries fee fields, because reth caps the estimate by that balance, and fills blob hashes from a request's sidecar | `src/api.rs` (`EthExtApi`) | ~50 | A configurable `ESTIMATE_GAS_ERROR_RATIO`, and no allowance cap for a request without `from` |
| Reports a failed block build as `failed to build payload <id>: missing payload`, because the payload service logs the build error and resolves with `MissingPayload`; anvil reports the EVM error | `src/miner.rs` | ~5 | Keep the job's error and return it from `resolve` |
| Replaces reth's engine validator to accept payload attributes with a timestamp at or below the parent's, and to convert a payload of a block before London to a header without a base fee, which the engine API cannot express | `src/engine.rs` | ~100 | A dev-mode switch for the timestamp check, and an engine API that carries an optional base fee |
| Replaces `eth_call` and `eth_estimateGas` to run a free call at any base fee, `eth_callMany` to answer every bundle and move each one a block past the one before, `eth_baseFee` to report the next-block override, `eth_sendRawTransactionSync` and `eth_sendTransactionSync` to report a timeout with code 4 and the hash, `eth_sendTransaction` to fall back to the largest gas limit when the estimate reverts, and `web3_clientVersion` to name this node | `src/api.rs` (`EthExtApi`, `Web3ExtApi`) | ~150 | Anvil's semantics for these are dev-node conveniences; a dev mode in reth could carry them |
| Serializes `eth_sendTransaction` and `eth_sendRawTransaction` per sender, from the nonce selection to the pool insertion, so concurrent requests without a nonce get distinct nonces: reth reads the next nonce and inserts without a lock | `src/api.rs` (`send`, `send_raw`) | ~30 | A per-sender lock around `send_transaction_request`, or a nonce reservation in the pool |
| Rejects a replacement whose fee does not exceed the pooled transaction's before it reaches the pool, because `PriceBumpConfig` with a zero bump replaces at an equal fee, and anvil requires a higher one; reth's default ten percent bump rejects anvil's `gas_price + 1` replacements | `src/api.rs` (`ensure_replacement_priced`), `src/pool.rs` | ~40 | A strict-inequality option on `PriceBumpConfig`, or a bump below one percent |
| Replaces the pending block environment builder so calls at `pending` see the next block's timestamp, coinbase, and prevrandao as the miner sets them; reth's `BuildPendingEnv` uses the parent timestamp plus twelve seconds | `src/pending.rs` | ~110 | A `PendingEnvBuilder` hook on `EthereumEthApiBuilder` |
| Chains automine blocks after a block that leaves ready transactions behind, waiting for the pool to see each block first, and groups transactions that arrive within five milliseconds into one block, as anvil's instant miner does; a block without transactions idles automine until the pool changes | `src/mining.rs`, `src/miner.rs` (`follow_up`, `MineIfPending`) | ~90 | A local miner mode that drains the pool |
| Gives the first block the genesis base fee through a one-shot override, where reth applies the EIP-1559 decrease of an empty parent; replaces `eth_feeHistory` to take the entry after the newest block from that block or from the next-block override, and to report a zero gas-used ratio for a block without a gas limit instead of NaN | `src/node.rs`, `src/api.rs` (`eth_fee_history`) | ~50 | A fee history that reads the child block; an initial base fee option for dev chains |
| Rejects at `eth_sendTransaction` and `eth_sendRawTransaction` a fee cap below the next block's base fee, as anvil does; reth's pool parks the transaction until the base fee drops. Rejects priced calls, estimates, and traces below the execution block's base fee, and a priced `eth_call` whose sender cannot pay for its gas and value; reth disables these checks for calls | `src/api.rs` (`ensure_fee_cap`, `ensure_call_fee_cap`, `ensure_call_funds`), `src/debug.rs` | ~100 | Pool and call options for these checks |
| Installs the `ArbSys` precompile on Arbitrum chains, through precompile builders that get the block number | `src/evm.rs` (`PrecompileBuilder`), `src/network/ethereum.rs` | ~15 | A block-aware precompile hook on `EvmFactory` |
| Gives impersonated transactions a signature with the sender in `r`, so the transactions of different impersonated senders get different hashes, as anvil's impersonated hash does | `src/impersonation.rs` | ~5 | A sender-attributed transaction that reth hashes with its sender |
| Replaces `eth_newFilter` to drain the block the filter is installed on, so the filter reports the blocks after it, as anvil's does; reth's first poll includes the install block. Gives a revert without data the empty `data` anvil reports | `src/api.rs` (`eth_new_filter`, `with_revert_data`), `src/node.rs` | ~30 | Install filters at the next block; `data: "0x"` on empty reverts |
| Replaces `eth_newBlockFilter` like `eth_newFilter`, `eth_getUncleCountByBlockHash` and `ByBlockNumber` to fail for an unknown block, `eth_signTransaction` to fill the chain id and the gas limit, `eth_signTypedData_v4` as an alias of `eth_signTypedData`, and `eth_getTransactionCount` at `pending` to answer from the pool and the latest state, because reth builds and caches its pending block for the lookup, and the cached block then misses the transactions of the next second | `src/api.rs` (`EthExtApi`) | ~90 | A pending nonce that does not build a block; `eth_signTypedData_v4` |
| Retries `eth_getTransactionReceipt` and `eth_getTransactionByHash` with the transaction's block in the RPC cache when the lookup failed to recover the sender from the signature: an impersonated transaction has no valid signature, and reth's disk path recovers instead of reading the senders table | `src/api.rs` (`cache_block_of`) | ~30 | Read `TransactionSenders` in the RPC lookups |
| `anvil_dropTransaction` removes the sender's later transactions, `anvil_setNonce` and `anvil_setBalance` tell the pool the new values, and a snapshot revert brings the pool back to the snapshot: the transactions mined since return, the ones sent since go | `src/api.rs` (`anvil_revert`, `restore_pool`, `sync_pool_account`) | ~110 | Pool hooks for dropping dependents and for a pool snapshot |
| Serves anvil's Beacon API routes (`/eth/v1/beacon/blobs/{block_id}`, `/eth/v1/beacon/genesis`) from a tower layer in front of the JSON-RPC server | `src/beacon.rs` | ~250 | Reth has no Beacon API; a blob route on the RPC server would do |
| Installs BSC's P256 verifier when Haber is active and overrides `ecrecover` for the signatures `anvil_impersonateSignature` registers, from the EVM factory | `src/evm.rs` (`install`, `cheat_ecrecover`) | ~60 | A chain-aware precompile hook on `EvmFactory` |
| Installs an alloy `CryptoProvider` when a node first sets a signature override, so EIP-7702 authorities with an overridden signature recover to the override's address on every path, as anvil does: alloy-evm recovers authorities when it builds the transaction environment, before any anvil hook sees them. The provider is process-wide, so in one process the overrides of every node apply | `src/impersonation.rs` (`OverrideCryptoProvider`) | ~90 | A recovery hook in alloy-evm's `FromRecoveredTx`, or signed authorities in the transaction environment |
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
| Drops reth's cached pending block after a change to the pool, the state, or the next block's environment: reth reuses a pending block for a second and keys it on the parent alone, and anvil's pending block shows a change at once. Calls, estimates, and access lists at `pending` run with the pending block's state changes as state overrides: reth runs them with the pending block's environment on the latest state. `eth_estimateGas` without a block estimates on the pending block, as anvil does | `src/api.rs` (`PendingReset`, `with_pending_state`), `src/node.rs` | ~150 | A pending block cache that the node can invalidate, and pending calls on the pending block's state |
| Replaces `ots_getInternalOperations` to include the top-level operation, `ots_getBlockTransactions` to page from the first transaction, and implements `ots_searchTransactionsBefore` and `ots_searchTransactionsAfter`, which reth leaves unimplemented, over the node's own `trace_block` and `eth_*` methods | `src/otterscan.rs` | ~300 | The same in reth's Otterscan module |
| Runs a blob call without a blob fee cap at a zero blob base fee in `eth_call` and `trace_call`, as geth and anvil do, through a block override | `src/simulate.rs` (`with_zero_blob_base_fee`), `src/debug.rs` | ~30 | The same rule in reth's call path |
| Accepts a transaction whose priority fee is above its fee cap on Arbitrum chains, in the pool and in the EVM, as anvil does: Arbitrum does not enforce the EIP-1559 ordering | `src/pool.rs`, `src/evm.rs` (`EvmSettings`) | ~15 | A chain-aware fee rule in reth's pool validator |
| Answers a state read at a block below the `--prune-history` window with anvil's `BlockOutOfRangeError`, from an RPC middleware: reth keeps every state | `src/history.rs` | ~150 | A history pruning mode in reth with anvil's error |
| Leaves the console precompile out of `eth_config` | `src/node.rs` | ~15 | An `eth_config` hook for node-specific precompiles |

## Gaps that are not hooks

These are not yet implemented here and do not need a reth change to be implemented, but would also
be free if reth had a dev mode:

- `--disable-block-gas-limit` sets the block gas limit to `u64::MAX` instead of only skipping the
  check, because reth's payload builder and pool enforce the header's limit.
- `--disable-min-priority-fee` leaves `eth_maxPriorityFeePerGas` and `eth_feeHistory` to reth's
  gas price oracle; only `eth_gasPrice` drops the tip, as in anvil.
- `--max-persisted-states` and `--transaction-block-keeper` are accepted and have no effect: reth
  keeps the full history on disk, which is what these flags bound in anvil's memory.
  `--prune-history` keeps the full history too, but rejects state reads below its window, as anvil
  does.
- `eth_getFilterChanges` does not report the logs a reorg or a snapshot revert removed with
  `removed: true`; anvil and geth do.
- A `genesis.json` header carries the forks the node runs and the root of the whole genesis
  state; anvil's header carries the JSON config's forks and the state root of the alloc alone, so
  the genesis hash differs.
- Revm clears the storage of an account without balance, nonce, or code at the end of a block,
  so `anvil_setStorageAt` on such an account does not stick; anvil keeps the storage.
- `eth_getProof` proves the state of the latest block; anvil's proofs include the state writes it
  has not mined yet.
- Reth builds the pending block without anvil's check of the deposit contract's logs, so a
  malformed deposit log does not fail `eth_getBlockByNumber("pending")`.
- Anvil's precompile factory (`NodeConfig::with_precompile_factory`) is not served.
- Reth's pool requires the sidecar of a blob transaction; anvil mines a blob transaction sent
  without one. A blob call with a zero blob fee cap keeps the block's blob base fee in
  `eth_simulateV1` on reth, which rejects the call when validation is off; anvil runs it at zero.
- `eth_simulateV1` shows the maximum nonce on a transaction that ran with it and no validation,
  as anvil does, but the transaction ran with nonce zero: revm cannot execute the maximum nonce.
- A trace replays the transaction with the precompiles of the node's current chain id, so after
  `anvil_setChainId` switches to a chain with other precompiles, the traces of the blocks mined
  before it change; anvil serves the traces it recorded at mining. Replays after a chain id
  change skip the EVM's chain id check, and the API checks the chain id of a request instead.
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
  `ethereum-optimism/optimism` and must be pinned to the same reth revision as this crate; see
  `docs/networks.md`. Tempo runs (`src/network/tempo.rs`); see "What Tempo needs". Monad is not
  run: networks other than Ethereum belong in extensions their teams own, on the `AnvilNetwork`
  API.

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
- Arbitrum forks number blocks by the L2 block; anvil mirrors Arbitrum's L1 block numbers in
  `NUMBER` and in the blocks' `l1BlockNumber`.

## Anvil's own tests

`tests/it/{anvil_api,api,transaction,gas,revert,logs,filter,pubsub,sign,txpool,genesis,proof,
block_index,storage_values,eip2935,eip4844,eip6110,eip7702,eip7928,otterscan,beacon_api,ipc,wsapi,
anvil,traces,simulate,state,fork,fork_bal,fork_chains,tempo,tempo_canary}.rs` are anvil's modules of the same name with the in-process calls made async and the anvil-internal hooks removed
(`api.backend`, `api.execute`, pool types, `eth_callBundle`, the ready-transaction listener, the
state dump's transaction records, the fee manager's blob fee, the Optimism variants, the block
listener count, the precompile factory, the JavaScript tracer). Ignored tests carry the reason on the attribute. Assertions on wall-clock seconds
became lower bounds, because blocks take longer to build here than on anvil, error messages
compare case-insensitively where reth's text differs only in case, and accept reth's text where
it differs. `state.rs`'s history pruning tests are ignored, because reth keeps the full history.
`fork_bal.rs` keeps the assertions on the state a fork serves and drops the ones on the fork
cache, because the node reads remote state lazily and has no block access list prefill; its
tests that need a deterministic block hash on a replacement endpoint start the endpoints from
the same genesis. `eth_config`'s blob schedule compares as JSON, because the in-process API
round-trips through EIP-7910's fields.

## Differences that cast's tests show

The workspace's `anvil` dependency is this crate. `crates/cast/tests` pass, Tempo included,
except these groups. None of them is a missing method. Revert messages decode the revert data
with foundry's `RevertDecoder`, as anvil's do.

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

- Amsterdam state gas. Anvil does not charge EIP-8037 state gas (foundry-rs/foundry#17428);
  this node does, so a transaction on Amsterdam that creates state with a tight gas limit runs
  out of gas here. Tests that pin such gas limits need more gas.

`crates/forge/tests/cli` pass, except a `--fork-bal` test: anvil's block access lists carry no storage reads, reth's do, and forge's
parent cache then does not fall back to the endpoint for read-only slots.

The unit tests of `forge-script` and `foundry-evm-core` that spawn a node pass as well, except
those that need Optimism, and the `--fork-bal` prewarm test, for EIP-8037 above.

## What Tempo needs

Tempo runs (`src/network/tempo.rs`) on Tempo's node types, primitives, pool implementation, eth
API, and block executor, with `AnvilEvmConfig<TempoEvmConfig>` as the EVM config, and without a
change to tempo (rev `6ef1f812`). These are the workarounds, and the tempo hook that would remove
each.

| Workaround in reth-anvil | Tempo hook that removes it |
| --- | --- |
| A dev block builder (`tempo_payload.rs`): sequential, without prewarming, parallel replay, or build budgets. `TempoPayloadBuilderBuilder` takes `TempoEvmConfig` as a concrete type (`crates/node/src/node.rs:885`). | Builders generic over the EVM config, as reth's `EthereumPayloadBuilder` is. |
| A pool builder that repeats `TempoPoolBuilder::build_pool` (`node.rs:772`) for the wrapped config. | The same. |
| An assembler adapter around `TempoBlockAssembler`, which implements only `BlockAssembler<TempoEvmConfig>` (`crates/evm/src/assemble.rs:86`). | An assembler over any config whose block executor factory is Tempo's. |
| No `console.log` and no fork block hashes below the fork block: `TempoEvmConfig::new` builds `TempoEvmFactory::default()`, so the anvil EVM factory cannot wrap it. The `ecrecover` override goes in through `AnvilEvmConfig::evm_with_env` instead, on the EVM the factory created. | A constructor that takes an EVM factory. |
| A copy of `TempoEthApi` (`tempo_eth.rs`, about 300 lines): Tempo's returns `PendingBlockKind::None`, reports `NATIVE_BALANCE_PLACEHOLDER` for native balances (`crates/node/src/rpc/mod.rs:351`), and simulates every AA call with a zero hash and one shared identifier (`crates/alloy/src/rpc/revm_compat.rs:68,94`), so two expiring nonce calls in one bundle collide. The copy builds pending blocks, reports balances, and hashes each simulated expiring nonce call by its request. A fork reads accounts with `eth_getAccountInfo`, as anvil does. | A pending block kind and a balance policy on `TempoEthApi`, and a unique identifier per simulated call. |
| A pool-only EVM config (`TempoPoolEvmConfig`): the pool validates against the block anvil mines next, at anvil's clock, and skips the fee balance check when pool balance checks are off. The validator still bounds `valid_before` by the tip timestamp (`crates/transaction-pool/src/validator.rs:210`), and `valid_after` by the wall clock, which reth-anvil lifts and checks against anvil's clock at its API instead. | A clock the node can give the validator. |
| A pool refresh after anvil state writes: the validator keeps the state it read at the tip until the next block (`validator.rs:399`), so reth-anvil replays the tip to the pool. The 2D nonce pool still learns lane changes only from blocks. | A way to drop the validator's read cache. |
| Calls, estimates, and access lists run with the request's nonce through a state override: reth drops the request's nonce for calls (`crates/rpc/rpc-eth-api/src/helpers/call.rs:895`), and Tempo charges a new account's cost to nonce zero. | A reth option to keep the request's nonce. |
| Simulated and sent call batches keep no create target: reth's `resolve_transaction` and anvil's request filling mark a request without `to` as a creation, which adds a create call to a Tempo batch. | A reth hook for the default kind of a request. |

Behavior that follows Tempo instead of anvil's emulation of it:

- Pool errors carry the text of Tempo's validator, for example `value transfer not allowed`
  instead of `native value transfer not allowed in Tempo mode`, `invalid chain ID` instead of
  `invalid chain id for signer`, and Tempo's intrinsic gas and `valid_before` messages. A fee
  token shortfall keeps anvil's `insufficient fee token balance` message.
- The dev chain's epoch length is `u64::MAX`, so no block ends an epoch: from T8 on, the last
  block of an epoch must carry a key generation outcome, which a dev chain has none of.

## Things reth already fixes

Bugs open on anvil that reth-anvil does not have, because the logic is reth's:

- foundry-rs/foundry#17428: `eth_estimateGas` on Amsterdam returns 21,000 for a transfer that creates
  an account. Reth executes the transfer at 21,000 gas and only returns that estimate when the run
  succeeds. Test: `estimate_gas_charges_account_creation_state_gas_on_amsterdam`.
