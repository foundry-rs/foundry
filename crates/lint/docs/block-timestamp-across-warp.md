# Timestamp captures across warp

**Severity**: `Med`
**ID**: `block-timestamp-across-warp`

## What it does

Warns in tests and scripts when a value derived from `block.timestamp` can be used after
`vm.warp`, or when `block.timestamp` is read both before and after a warp in the same call.

Indirect cheatcode calls, inline assembly, and values stored through heap or storage aliases
may be missed. Use the getter for captures that must survive a warp even without a warning.

## Why is this bad?

The EVM timestamp is constant during a normal transaction. Compilers can therefore reuse,
move, or defer `block.timestamp` reads across calls. Foundry's `vm.warp` changes that environment
inside a test, so a Solidity local is not a guarantee that the earlier timestamp was captured.

Use `vm.getBlockTimestamp()` for test values that need to observe a particular point in time.
The getter captures the value at the time of the call.

## Example

```solidity
vm.warp(100);
uint256 saved = block.timestamp; // Warning: this capture can cross the next warp.
vm.warp(200);
vm.warp(saved);
```

Use instead:

```solidity
vm.warp(100);
uint256 saved = vm.getBlockTimestamp();
vm.warp(200);
vm.warp(saved); // Restores 100.
```

For before/after comparisons, use the getter on both sides of the mutation.
