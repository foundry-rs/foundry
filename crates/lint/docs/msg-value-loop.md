# Msg value inside loop

**Severity**: `Low`
**ID**: `msg-value-loop`

Flags `msg.value` reads inside loops reachable from externally callable payable functions.

## What it does

Reports `msg.value` expressions that execute inside a `for`, `while`, or `do while` loop
reachable from a `public payable` or `external payable` entry point. This includes loops
introduced by modifiers and reads inside helpers transitively called from the entry point.

Payable constructors are ignored. `receive()` and `fallback()` functions are checked when they are
payable.

## Why is this bad?

`msg.value` is fixed for the whole transaction. Reading it inside a loop can accidentally treat
the same Ether payment as if it were supplied once per iteration.

This can lead to incorrect accounting, repeated credits, or fund loss when loop iterations send,
record, or otherwise consume value based on `msg.value`.

## Example

### Bad

```solidity
function batch(address[] calldata receivers) external payable {
    for (uint256 i; i < receivers.length; ++i) {
        credits[receivers[i]] += msg.value;
    }
}
```

### Good

```solidity
function batch(address[] calldata receivers) external payable {
    uint256 share = msg.value / receivers.length;
    for (uint256 i; i < receivers.length; ++i) {
        credits[receivers[i]] += share;
    }
}
```

## Notes

Review each occurrence manually. Prefer computing the intended per-iteration amount before the
loop, then use that derived value inside the loop.

Inline assembly is not inspected. Calls through function pointers are not followed.
