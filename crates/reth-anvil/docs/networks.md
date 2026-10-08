# Adding a network to reth-anvil

reth-anvil runs a reth node and adds anvil's controls on top: mining, time, state writes,
impersonation, snapshots, forks, and the `anvil_*` namespace. A network supplies the reth node
types and the components that differ from Ethereum. Everything else is shared.

This guide is for the teams that own a network, for example Optimism, Base, and Monad. Ethereum
(`src/network/ethereum.rs`) and Tempo (`../reth-anvil-tempo/src/tempo.rs`) are the worked examples.
Tempo implements the SDK from a separate crate. `reth-anvil-cli` owns the `anvil` binary and
selects between the two implementations. The core SDK has no direct Tempo dependency.

## What a network implements

A network is a marker type that implements `AnvilNetwork` (`src/network/mod.rs`):

| Item | What it is |
| --- | --- |
| `Node` | The reth node type. It sets the primitives, the chain spec, and the payload types. |
| `Fork` | The `ForkNetwork` adapter for remote responses and checkpoint headers. It is owned by the network, rather than its foreign primitive types. |
| `Components` | The node components builder. Install the anvil wrappers here; see below. |
| `AddOns` | The RPC add-ons. Their `EthApi` must serve the network's RPC types. |
| `Attributes` | The payload attributes builder for the next block. |
| `prepare` | Builds the chain spec and, for a fork, the fork backend. It can change the config, for example to adopt the chain id of the fork. |
| `components`, `add_ons`, `payload_attributes` | Build the items above from the shared anvil state (`AnvilComponents`). |
| `extend_rpc` | Registers network methods with their own request types and can replace standard handlers. The launcher installs them on every transport and rebuilds them after reset. |
| `identity` | What `anvil_nodeInfo` reports: the network name and the hardfork. |
| `FIRST_BLOCK_KEEPS_GENESIS_BASE_FEE` | Set it to `false` when the chain spec sets every base fee, as Tempo's does. |

The network's types implement small traits, so the shared code can use them:

| Trait | Implemented by | Purpose |
| --- | --- | --- |
| `AnvilNextBlockEnv` | the EVM config's next block context | Anvil sets the timestamp, coinbase, prevrandao, gas limit, and beacon root of the next block. |
| `AnvilPayloadAttributes` | the payload attributes | The miner sets the same fields on the payload. |
| `AnvilExecutionPayload` | the execution payload | The executor finds impersonated senders by transaction. |
| `ConsoleEvmFactory` | the EVM factory | `console.log` printing. Return `None` when the factory has no console. |
| `ForkNetwork` | a fork marker type | Convert remote blocks, receipts, and transactions into the node's types. |
| `TxPoolKey` | the signed transaction | The key of a transaction in `txpool_content` and `txpool_inspect`. |
| `CallBatch` | the RPC transaction request | Call batches, request validation, nonce state overrides, and fields that a signature covers. |

The adapter traits take the local network marker as a type parameter, for example
`impl CallBatch<Tempo> for TempoTransactionRequest`. This lets an external crate implement the
traits for its upstream types under Rust's orphan rules. Pass the same marker to
`AnvilExecutorBuilder`, `AnvilPendingEnv`, and other wrappers that use these adapters.

`NodeConfig::extensions` stores typed, cloneable settings from the network crate. Tempo provides
`TempoConfigExt` for its builders and hardfork resolution. Import that trait when using
`NodeConfig::test_tempo()` or the fee-payer builders.

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
them. Tempo is that case. It builds its pool and its blocks in reth-anvil-tempo, and `UPSTREAM.md`
lists the hooks that would remove that code. Prefer to add the hook to the network's crates.

## Steps

1. Create a crate that depends on `reth-anvil` and the network's upstream crates.
2. Implement `AnvilNetwork` and the adapters above in that crate. Keep the same reth, revm,
   and alloy-evm revisions as the SDK.
3. Use `AnvilExecutorBuilder::new(inner, anvil)` to reuse the node's authoritative state,
   clock, and impersonation handles. If a foreign trait requires a local EVM newtype, delegate
   `evm_with_env`, `evm_with_env_and_inspector`, and `tx_iterator_for_payload` as well as the
   required methods. These overrides preserve impersonation and sender-cache recording.
4. Register network RPC methods through `extend_rpc`. Use `AnvilRpc::eth_api()` for the native
   provider and pool, and `AnvilRpc::update_state()` to apply writes and invalidate their caches.
   Return a `RpcModule<()>`; matching method names replace the shared handlers.
5. Launch the network with `launch::<YourNetwork>(config)`. An application that selects between
   networks should depend on each extension and dispatch once at its boundary. Use
   `NodeConfig::resolve_networks_for` with the installed networks for implicit fork discovery.
6. Keep network tests in the extension crate. Put tests that invoke the executable in the CLI
   crate. Ethereum and the ignored Optimism/Base specifications remain in the SDK crate.
7. Run tests for the core, extension, and binary together. For the installed networks:
   `cargo nextest run -p reth-anvil -p reth-anvil-tempo -p reth-anvil-cli`.

The binary moved from `reth-anvil` to `reth-anvil-cli`. Build or install the latter package when
an executable is needed. The workspace's `anvil` dependency points to that facade, so Foundry
callers retain runtime network selection. SDK callers can depend on core alone and launch a
concrete network without the CLI.

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
