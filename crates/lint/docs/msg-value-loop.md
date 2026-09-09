# `msg.value` inside a loop

**Severity**: `Low`
**ID**: `msg-value-loop`

Flags `msg.value` reads inside loops reachable from externally callable payable functions.

## What it does

Reports `msg.value` expressions that execute inside a `for`, `while`, or `do while` loop
reachable from a `public payable` or `external payable` entry point.

Payable constructors are ignored. `receive()` and `fallback()` functions are checked when they are
payable.

## Why is this bad?

`msg.value` is fixed within a call frame. Nested calls can carry different values, while
`delegatecall` preserves the calling frame's value. Reading it inside a loop can accidentally
treat the same Ether payment as if it were supplied once per iteration.

This can lead to incorrect accounting, repeated credits, or fund loss when loop iterations send,
record, or otherwise consume value based on `msg.value`.

## Example

```solidity
function batch(address[] calldata receivers) external payable {
    for (uint256 i; i < receivers.length; ++i) {
        credits[receivers[i]] += msg.value;
    }
}
```

Use instead:

For an equal split, reject an empty recipient list and choose how to handle any remainder. This
example accepts only exactly divisible payments; other APIs may explicitly refund or account for
the remainder.

```solidity
function batch(address[] calldata receivers) external payable {
    require(receivers.length != 0, "no receivers");
    require(msg.value % receivers.length == 0, "unequal split");
    uint256 share = msg.value / receivers.length;
    for (uint256 i; i < receivers.length; ++i) {
        credits[receivers[i]] += share;
    }
}
```

## Notes

Review each occurrence manually. Prefer computing the intended per-iteration amount before the
loop, then use that derived value inside the loop.
