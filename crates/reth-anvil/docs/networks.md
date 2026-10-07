# Adding a network to reth-anvil

reth-anvil runs a reth node and adds anvil's controls on top: mining, time, state writes,
impersonation, snapshots, forks, and the `anvil_*` namespace. A network supplies the reth node
types and the components that differ from Ethereum. Everything else is shared.

This guide is for the teams that own a network, for example Optimism and Base. Ethereum
(`src/network/ethereum.rs`), Monad (`src/network/monad.rs`), and Tempo (`src/network/tempo.rs`)
are the worked examples. Monad shows a network on Ethereum's node types with its own EVM. Tempo
shows a network with its own node types, pool, and RPC.

## What a network implements

A network is a marker type that implements `AnvilNetwork` (`src/network/mod.rs`):

| Item | What it is |
| --- | --- |
| `Node` | The reth node type. It sets the primitives, the chain spec, and the payload types. |
| `Components` | The node components builder. Install the anvil wrappers here; see below. |
| `AddOns` | The RPC add-ons. Their `EthApi` must serve the network's RPC types. |
| `Attributes` | The payload attributes builder for the next block. |
| `prepare` | Builds the chain spec and, for a fork, the fork backend. It can change the config, for example to adopt the chain id of the fork. |
| `components`, `add_ons`, `payload_attributes` | Build the items above from the shared anvil state (`AnvilComponents`). |
| `identity` | What `anvil_nodeInfo` reports: the network name and the hardfork. |
| `FIRST_BLOCK_KEEPS_GENESIS_BASE_FEE` | Set it to `false` when the chain spec sets every base fee, as Tempo's does. |

The network's types implement small traits, so the shared code can use them:

| Trait | Implemented by | Purpose |
| --- | --- | --- |
| `AnvilNextBlockEnv` | the EVM config's next block context | Anvil sets the timestamp, coinbase, prevrandao, gas limit, and beacon root of the next block. |
| `AnvilPayloadAttributes` | the payload attributes | The miner sets the same fields on the payload. |
| `AnvilExecutionPayload` | the execution payload | The executor finds impersonated senders by transaction. |
| `ConsoleEvmFactory` | the EVM factory | `console.log` printing. Return `None` when the factory has no console. |
| `ForkNetwork`, `AnvilPrimitives` | a fork marker type, the primitives | Convert remote blocks, receipts, and transactions into the node's types. |
| `TxPoolKey` | the signed transaction | The key of a transaction in `txpool_content` and `txpool_inspect`. |
| `CallBatch` | the RPC transaction request | Call batches, nonce lanes, and fields that a signature covers. Keep the defaults on an Ethereum-shaped request. |

## The anvil wrappers

Install these in `components`, as `ethereum.rs` does:

- `AnvilExecutorBuilder` wraps the network's executor builder. It applies anvil's state writes,
  the transaction cap per block, the block environment overrides, and impersonated senders. It
  takes any EVM config whose EVM factory implements `ConsoleEvmFactory`.
- `AnvilEvmFactory` wraps the network's EVM factory. It adds anvil's precompiles, the
  `console.log` precompile, `ecrecover` with signature overrides, and fork block hashes. Use it
  when the network's EVM config takes an EVM factory, as `EthEvmConfig::new_with_evm_factory`
  does.
- `AnvilPoolBuilder` builds an Ethereum-shaped pool with anvil's validator: impersonated senders,
  anvil's balance rule, and anvil's transaction order. A network with its own pool builds it
  with the wrapped EVM config, as `TempoAnvilPoolBuilder` does.

When the network's builders take its own EVM config as a concrete type, the wrappers cannot reach
them. Tempo is that case. It builds its pool and its blocks in reth-anvil, and `UPSTREAM.md`
lists the hooks that would remove that code. Prefer to add the hook to the network's crates.

## Steps

1. Add a Cargo feature to `crates/reth-anvil/Cargo.toml` with the network's crates as optional
   dependencies. Put a comment with the feature name above the group.
2. Add `src/network/<name>.rs` behind the feature, and implement the items above.
3. Dispatch to the network in `try_spawn` (`src/node.rs`), and return `true` for it in
   `runs_network` (`src/config.rs`). The network selection, the hardfork parsing, and the fork
   endpoint discovery are in `foundry-evm-networks` and `foundry-evm-hardforks` already.
4. Add the network's hardfork to `NodeConfig`, as `get_tempo_hardfork` and `get_monad_hardfork`
   do, and map it to the Ethereum hardfork it runs on in `ethereum_hardfork_at`.
5. Add the network's anvil methods to the `anvil_*` namespace (`src/api.rs`), and fail them with
   `Not implemented` on other networks, as the Tempo methods do.
6. Port anvil's tests for the network to `tests/it/<name>.rs` behind the feature. Make the calls
   async, use the RPC instead of anvil's internal backend, and give every ignored test its reason.
7. Run `cargo nextest run -p reth-anvil --features <name>`, and the same without the feature.

## Optimism and Base

Anvil's Optimism and Base tests are in `tests/it/optimism.rs` and `tests/it/base.rs`. They are the
specification for these networks. Every test is ignored until the network runs.

The node types come from op-reth, which moved from the reth repository to
`ethereum-optimism/optimism`. reth-anvil pins reth at `5723a3f` (v2.5.2) with revm 43 and
alloy-evm 0.39, so op-reth must build against the same reth revision:

- `ethereum-optimism/optimism` on its default branch uses revm 42 and does not build with this
  crate.
- The `foundry-rs/optimism` fork at `206d26` uses revm 43 and alloy-evm 0.39, but pins reth at
  `c12b8f2`, 304 commits before `5723a3f`. A local bump of that pin to `5723a3f` found one
  conflict: `alloy-eips` 2.4.1 in its lockfile against 2.5 here.

Expected shape: `OpNode` as `Node`, `OpEvmConfig` built with `AnvilEvmFactory` around the OP EVM
factory, OP's pool with the wrapped config, and OP's payload builder. Deposit transactions,
the L1 and operator fees, and the OP hardforks must behave as anvil's Optimism mode does; the
ported tests check that. Base adds its own transaction type (EIP-8130) and upgrades on top of the
OP stack, from the `base-common-*` crates.
