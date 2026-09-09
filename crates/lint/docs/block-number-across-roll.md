# Block number captures across roll

**Severity**: `Med`
**ID**: `block-number-across-roll`

## What it does

Warns when a value derived from `block.number` can be used after `vm.roll`, or when raw
block-number reads occur on both sides of a roll in the same call frame.

## Why is this bad?

The EVM block number is constant during a normal transaction. Compilers can therefore reuse,
move, or defer `block.number` reads across calls. Foundry's `vm.roll` changes that environment
inside a test, so a Solidity local is not a guarantee that the earlier number was captured.
This can happen with optimized solc via IR as well as other compiler optimizations.

Use `vm.getBlockNumber()` for test values that need to observe a particular point in time.
The getter captures the value at the time of the call. Keep normal compiler optimizations enabled.

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

## Scope and controls

The rule applies to tests and scripts, regardless of optimizer settings. Ordinary
`block.number` reads without `vm.roll` are not flagged.

Use `vm.getBlockNumber()` whenever a test needs to save a block number across `vm.roll`;
the absence of a warning does not guarantee that a raw capture is reliable.

Existing severity filters, `exclude_lints`, and inline suppressions apply. Suppress at the raw
capture when its behavior is intentional:

```solidity
// forge-lint: disable-next-line(block-number-across-roll)
uint256 saved = block.number;
```
