# evm2 execution

Foundry executes every supported network family on [evm2](https://github.com/alloy-rs/evm2). The
execution contracts live in Rustdoc next to their owners:

- Network families: [`FoundryEvmNetwork` and `FoundryEvmFactory`](../../crates/evm/core/src/evm/mod.rs),
  with network-native conversion in [`FoundryTxConversion`](../../crates/evm/core/src/evm/tx.rs)
  and running-state synchronization in [`FoundryExecutionHooks`](../../crates/evm/core/src/evm/env.rs).
  Selection and dispatch follow [Custom EVM integrations](./networks.md).
- Backend and fork state: [`Backend`](../../crates/evm/core/src/backend/mod.rs) for the execution
  contract between the backend, the EVM and cheatcode inspectors, and
  [`ExecutionLifecycle`](../../crates/evm/core/src/evm/lifecycle.rs) for the accepted state of
  forks, persistent accounts, snapshots and nested transactions on top of evm2's `State`.
- Inspector stack and isolation: [`InspectorStack`](../../crates/evm/evm/src/inspectors/stack.rs),
  including the nested-transaction roots of isolated calls, `vm.executeTransaction` and
  `vm.transact`.
- Step sets: [`Cheatcodes::steps`](../../crates/cheatcodes/src/inspector.rs) decides which opcodes
  the cheatcodes step; the inspector stack's `inspector_config` combines it with the other
  inspectors' needs.

`foundry-evm-core` still depends on `revm` only for the block environment that RPC fork caches
serialize, so existing caches stay readable. revm does not execute anything.

## Known divergences from revm

These compatibility differences affect execution, gas reports and Anvil library integrations.

- **Recipient balance overflow**: evm2's `State::transfer` saturates the recipient's balance at
  `type(uint256).max` instead of halting with revm's `OverflowPayment`. Overflow alone does not
  fail the call: the sender loses the full value and the excess is burned if the transfer commits.
  The symbolic engine still models revm's failure.
- **Arbitrary storage warmth**: with `vm.setArbitraryStorage`, the revm inspector warmed slots
  before `SLOAD`. evm2 leaves otherwise-cold slots cold until the opcode accesses them, costing
  2,000 more gas per cold read on Berlin and later.
- **Fork warmth**: warmth resets between transactions instead of leaking from `setUp` through
  revm's retained fork journals. Within a transaction, forks retain their own warmth and persistent
  accounts carry warmth across switches. Isolated calls are separate transactions; an isolated
  call that switches forks can replace the outer transaction's warmth with its own. Transient
  storage follows the same lifetime, so it no longer leaks from `setUp` into a test through a fork.
  See [`ForkSwitch.t.sol`](../../testdata/default/cheats/ForkSwitch.t.sol).
- **`vm.transact` cheatcode access**: accounts granted `vm.allowCheatcodes` retain access inside
  `vm.transact`; revm's fresh replay backend did not preserve those permissions.
- **Amsterdam isolation**: isolated `CREATE`s use the creator's current nonce for EIP-8037
  account-creation gas, as non-isolated creates do. revm reused the outer transaction's nonce and
  could skip that charge after the first creation.
- **`vm.setEvmVersion`**: new calls, creates and executed transactions use the requested spec;
  running frames keep theirs, unlike revm's opcode-level changes. revm also reported 6,000 more
  gas per test after `vm.setEvmVersion("amsterdam")` in `setUp`, even after switching back, because
  its stipend computation kept the EIP-2780 flag set.
- **Anvil `PrecompileFactory`**: library users must enumerate custom precompiles as
  `(Address, PrecompileId, DynPrecompile)` entries. Anvil no longer exposes `install` for customizing
  the precompile map or adding dynamic address lookup; this is an Anvil API limitation, not a
  limitation of evm2's `PrecompileProvider`.
