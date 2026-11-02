# evm2 execution

Foundry executes every supported network family on [evm2](https://github.com/alloy-rs/evm2). The
execution contracts live in Rustdoc next to their owners:

- Network families: [`FoundryEvmNetwork` and `FoundryEvmFactory`](../../crates/evm/core/src/evm/mod.rs),
  with network-native transactions in [`evm::tx`](../../crates/evm/core/src/evm/tx.rs). Selection
  and dispatch follow [Custom EVM integrations](./networks.md).
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

Foundry releases before evm2 executed on revm. These differences are accepted: do not adjust gas
snapshots or tests to reproduce the revm behavior.

- **Recipient balance overflow** ([OSS-1253](https://linear.app/tempoxyz/issue/OSS-1253)): a value
  transfer that would overflow the recipient's balance succeeds on evm2. The sender loses the value
  and the recipient's balance saturates at `type(uint256).max`. revm failed the transfer without
  changing balances. It is reachable only through `vm.deal` or `anvil_setBalance`, and concrete
  execution disagrees with the symbolic engine, which models revm's halt.
- **Arbitrary storage warmth**: revm read every slot warm after `vm.setArbitraryStorage`. On evm2,
  slots of an arbitrary-storage account warm on first access as usual, so a first read costs 2,000
  more gas.
- **Fork warmth across transactions**: revm kept a journal per fork across transactions, so state
  touched in `setUp` stayed warm after `vm.selectFork` or `vm.rollFork` in a test, and with
  `isolate` its `vm.rollFork` cooled slots the test had already read. On evm2, warmth is per
  transaction and survives fork switches within it in both isolation modes (`Issue3110`,
  `Issue3220`, `Issue5739`, `Issue8006`, `PersistentWrites`).
- **`vm.transact` cheatcode access**: revm ran `vm.transact` over a fresh backend that denied
  cheatcodes to `vm.allowCheatcodes` accounts. On evm2 those accounts can call cheatcodes inside
  `vm.transact`, as in other calls.
- **Amsterdam isolation**: revm never set the nonce of an isolated call's transaction, so isolated
  `CREATE`s after the first skipped EIP-8037 account-creation state gas. On evm2, every isolated
  `CREATE` pays it, as a non-isolated one does.
- **`vm.setEvmVersion`**: evm2 changes the spec only between frames, so a change applies from the
  next call or create, not from the next opcode. revm also reported 6,000 more gas per test after
  `vm.setEvmVersion("amsterdam")` in `setUp`, even after switching back, because its stipend
  computation kept the EIP-2780 flag set.
- **Anvil `PrecompileFactory`**: library users return a fixed `(Address, PrecompileId,
  DynPrecompile)` list. revm's `install` hook, which could customize the precompile map, has no
  equivalent.
