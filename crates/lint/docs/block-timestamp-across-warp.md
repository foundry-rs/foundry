# Timestamp captures across warp

**Severity**: `Med`
**ID**: `block-timestamp-across-warp`

## What it does

Warns when a value derived from `block.timestamp` can be used after `vm.warp`, or when raw
timestamp reads occur on both sides of a warp in the same call frame.

## Why is this bad?

The EVM timestamp is constant during a normal transaction. Compilers can therefore reuse,
move, or defer `block.timestamp` reads across calls. Foundry's `vm.warp` changes that environment
inside a test, so a Solidity local is not a guarantee that the earlier timestamp was captured.
This can happen with optimized solc via IR as well as other compiler optimizations.

Use `vm.getBlockTimestamp()` for test values that need to observe a particular point in time.
The getter captures the value at the time of the call. Keep normal compiler optimizations enabled.

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

## Scope and controls

The rule applies to tests and scripts, regardless of optimizer settings. Ordinary
`block.timestamp` reads without `vm.warp` are not flagged.

Use `vm.getBlockTimestamp()` whenever a test needs to save a timestamp across `vm.warp`;
the absence of a warning does not guarantee that a raw capture is reliable.

Existing severity filters, `exclude_lints`, and inline suppressions apply. Suppress at the raw
capture when its behavior is intentional:

```solidity
// forge-lint: disable-next-line(block-timestamp-across-warp)
uint256 saved = block.timestamp;
```

This is separate from `block-timestamp`, which asks you to review timestamp comparisons against
the target chain's timing guarantees. This rule concerns saved timestamps in tests and scripts.
