# Environment reads across Foundry mutations

**Severity**: `Med`
**ID**: `environment-read-across-mutation`

## What it does

Warns when a raw environment value can be used after a Foundry cheatcode changes that
environment, or when matching raw reads occur on both sides of a mutation in the same call
frame. Reads without a matching mutation are not flagged. The rule applies to tests and scripts,
regardless of optimizer settings.

## Why is this bad?

Solidity compilers can reuse, move, or defer environment reads that are invariant during
normal EVM execution. Foundry can change these values inside a test, so assigning a raw read
to a local does not guarantee that the compiler captures its value before a cheatcode call.
This affects optimized via-IR compilation as well as other compiler optimizations.

Use a getter or an external helper call to capture the value at the time of the call.
An internal helper can be inlined and does not provide this guarantee. For before/after
comparisons, use getters or external helpers on both sides of the mutation. Keep normal
compiler optimizations enabled.

## Example

In a test with the `vm` cheatcode interface:

```solidity
uint256 saved = block.chainid;
vm.chainId(2);
vm.chainId(saved); // The compiler need not have captured the original chain ID.
```

Use instead:

```solidity
uint256 saved = vm.getChainId();
vm.chainId(2);
vm.chainId(saved);
```

## Notes

These reads can be affected by the corresponding setters:

| Read | Direct setter | Getter or alternative |
| --- | --- | --- |
| `block.number` | `vm.roll` | `vm.getBlockNumber()` |
| `block.timestamp` | `vm.warp` | `vm.getBlockTimestamp()` |
| `block.chainid` | `vm.chainId` | `vm.getChainId()` |
| `block.coinbase` | `vm.coinbase` | External helper |
| `block.difficulty`, `block.prevrandao` | `vm.difficulty`, `vm.prevrandao` (both overloads) | External helper |
| `block.basefee` | `vm.fee` | External helper |
| `block.blobbasefee` | `vm.blobBaseFee` | `vm.getBlobBaseFee()` |
| `tx.gasprice` | `vm.txGasPrice` | External helper |
| `blockhash(n)` | `vm.setBlockhash`, `vm.roll` | External helper |
| `blobhash(i)` | `vm.blobhashes` | `vm.getBlobhashes()[i]` |
| `block.gaslimit`, `block.slotnum` (Amsterdam) | Fork changes and snapshot restoration | External helper |

`vm.roll` also affects `blockhash` because its valid history window depends on the current
block number. When replacing `blobhash(i)` with array indexing, handle out-of-range indices
if needed: the opcode returns zero, whereas indexing the getter's returned array reverts.

All overloads of `vm.selectFork`, `vm.createSelectFork`, and `vm.rollFork` are covered.
Fork changes replace block/configuration fields and blockhash history; switching forks also
switches fork-scoped gas-price and blob-hash overrides. `vm.revertToState` and
`vm.revertToStateAndDelete`, plus their deprecated `vm.revertTo` and `vm.revertToAndDelete`
aliases, can restore all listed environments. A warning does not establish that a particular
fork or snapshot operation changed the saved value; use a getter or external helper when
you need to retain it regardless of the selected fork or snapshot. Creating a fork without
selecting it does not change the current environment.

For fields without a getter, call a public/external helper externally:

```solidity
function baseFee() external view returns (uint256) {
    return block.basefee;
}

function example() public {
    uint256 saved = this.baseFee();
    vm.fee(2);
    vm.fee(saved);
}
```

`vm.prank` and `vm.broadcast` affect subsequent call frames, not the current frame's
`msg.sender`, `msg.value`, or `tx.origin` across the setter call. Balance, code, storage,
return-data, and gas reads are not in this invariant-environment class: the compiler already
accounts for their changes across calls.

This rule replaces `block-number-across-roll` and `block-timestamp-across-warp`. Update
`--only-lint`, `exclude_lints`, and inline suppressions to the new ID. Suppress at the raw
capture when its behavior is intentional:

```solidity
// forge-lint: disable-next-line(environment-read-across-mutation)
uint256 saved = block.chainid;
```

## Limitations

The absence of a warning does not guarantee that a raw capture is reliable, including in
assembly or when calling cheatcodes through low-level calls. Use getters or external helpers
when a test needs to retain an environment value across a matching mutation.
