# Block number captures across roll

**Severity**: `Med`
**ID**: `block-number-across-roll`

## What it does

Warns in tests and scripts when a value derived from `block.number` can be used after
`vm.roll`, or when `block.number` is read both before and after a roll in the same call.

Indirect cheatcode calls, inline assembly, and values stored through heap or storage aliases
may be missed. Use the getter for captures that must survive a roll even without a warning.

## Why is this bad?

The EVM block number is constant during a normal transaction. Compilers can therefore reuse,
move, or defer `block.number` reads across calls. Foundry's `vm.roll` changes that environment
inside a test, so a Solidity local is not a guarantee that the earlier number was captured.

Use `vm.getBlockNumber()` for test values that need to observe a particular point in time.
The getter captures the value at the time of the call.

## Example

```solidity
vm.roll(100);
uint256 saved = block.number; // Warning: this capture can cross the next roll.
vm.roll(200);
vm.roll(saved);
```

Use instead:

```solidity
vm.roll(100);
uint256 saved = vm.getBlockNumber();
vm.roll(200);
vm.roll(saved); // Restores 100.
```

For before/after comparisons, use the getter on both sides of the mutation.
