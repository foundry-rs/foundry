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

revm does not execute anything. Foundry's own code uses `revm` only for the block environment that
RPC fork cache metadata serializes. The `base` feature, which release builds enable, still links
revm 43 and alloy-evm 0.39 through `base-evm2` and the Base crates it depends on.

Gas consumers distinguish receipt charges from execution consumption. Monad uses
[`category-labs/monad-alloy-evm2`](https://github.com/category-labs/monad-alloy-evm2): ordinary
transactions charge their full gas limit, including on revert. Receipts, block accounting and
access-list RPC results retain that charge. Anvil estimates from `total_gas_spent` and verifies
the required limit by execution. Forge's result adapter reports Monad consumption while retaining
Ethereum's calldata floor and refunds. Monad script estimates still cover the required calldata
floor. Isolated frames pay their calldata floor rather than a full-limit receipt charge. The
native Monad factory leaves reserve-balance enforcement opt-in.

## User-visible output changes

- **Cheatcode `EXTCODEHASH`**: the cheatcode address, and any network-specific extra cheatcode
  addresses, hold the one-byte stub `0x00`, so `EXTCODEHASH` on them returns `keccak256(0x00)`.
  revm reported `keccak256(abi.encodePacked(CHEATCODE_ADDRESS))` for the cheatcode address and the
  hash of the address for extra ones.
- **Step-trace JSON**: steps in `forge test --json` traces no longer carry `gas_used`. A `CREATE`
  or `CREATE2` step's `gas_cost` now excludes the gas forwarded to the new frame. Each call's
  `step_deltas` lists `gas_remaining_after` for call-like steps and for steps whose remaining gas
  differs from `gas_remaining - gas_cost`.
- **`SLOAD` values**: with `-vvvvv --opcodes SLOAD`, every `SLOAD` shows its slot and loaded value,
  including repeated reads of a slot that revm printed as a bare `SLOAD`.
- **`prestateTracer` diff mode**: Anvil leaves every account that was empty before the transaction
  out of `pre`, also when the transaction funds it, such as the coinbase at its first fee credit.
  This matches geth. revm-inspectors reported such an account with `"balance":"0x0"`.

## Known divergences from revm

These compatibility differences affect execution, gas reports and Anvil library integrations.

- **Recipient balance overflow**: evm2's `State::transfer` saturates the recipient's balance at
  `type(uint256).max` instead of halting with revm's `OverflowPayment`. Overflow alone does not
  fail the call: the sender loses the full value and the excess is burned if the transfer commits.
  A `SELFDESTRUCT` beneficiary saturates the same way where revm wrapped it. A `CREATE` or
  `CREATE2` endowment instead wraps the new account's existing balance modulo 2^256 where revm
  failed the creation. The symbolic engine models the same behavior. The fee reward credited to
  the block beneficiary also wraps modulo 2^256, where revm left the balance unchanged when the
  credit overflowed: a coinbase that holds `type(uint256).max - 1` and receives a 21,000-gas
  transaction's 1 gwei tip holds 20,999,999,999,998 wei on evm2 and stays at `max - 1` on revm.
- **Arbitrary storage warmth**: with `vm.setArbitraryStorage`, the revm inspector warmed slots
  before `SLOAD`. evm2 leaves otherwise-cold slots cold until the opcode accesses them, costing
  2,000 more gas per cold read on Berlin and later.
- **Fork warmth**: warmth resets between transactions instead of leaking from `setUp` through
  revm's retained fork journals. Within a transaction, forks retain their own warmth and persistent
  accounts carry warmth across switches. Isolated calls are separate transactions; an isolated
  call that switches forks can replace the outer transaction's warmth with its own. Transient
  storage follows the same lifetime, so it no longer leaks from `setUp` into a test through a fork.
  See [`ForkSwitch.t.sol`](../../testdata/default/cheats/ForkSwitch.t.sol).
- **Fork environment on re-select**: switching back to a fork restores the whole block environment
  it last ran with, including `vm.fee`, `vm.coinbase` and `vm.prevrandao` changes. revm kept only
  the block number and timestamp and restored the rest from the fork block.
- **Invariant dictionary**: campaigns no longer step every opcode, so the dictionary no longer
  collects stack values seen around calls or types storage values by the mapping keys that produced
  them. Storage, call results, logs and bytecode literals are still collected.
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
- **Tempo blob transactions**: Anvil on Tempo rejects EIP-4844 transactions with "EIP-4844 blob
  transactions are not supported on Tempo", because Tempo's EVM has no blob transaction type.
- **Anvil empty accounts**: Anvil removes an account that a transaction touches and leaves empty, as
  EIP-161 requires. revm-based Anvil kept it in state, for example an OP Stack fee vault after a
  zero fee credit or the target of a call to an address without code. For the same transactions,
  state roots and block hashes differ from revm-based Anvil.
- **Anvil state dumps**: dumps written by this version do not load on revm-based Anvil. They store
  historical-state code as raw bytes and trace steps without `gas_used`, which older versions cannot
  deserialize. They also leave out zero-valued storage slots.
- **Fork caches**: RPC fork caches written by revm-based Foundry are reused when their block
  environment metadata matches. Tempo caches, whose metadata revm wrote as Tempo's block
  environment, and caches of blocks without a `mixHash` that revm recorded with no `prevrandao` no
  longer match, so they are discarded and refetched. This version writes account code as raw bytes,
  so older versions discard the caches it writes.
