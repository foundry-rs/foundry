# Unsafe typecast

**Severity**: `Med`
**ID**: `unsafe-typecast`

Flags explicit numeric typecasts that can silently truncate or alter the value.

## What it does

Reports casts where the source value's type can exceed the target type (for example,
`uint256 → uint128` or `int256 → uint128`). An unsigned value masked to the target width, such
as `uint8(value & 0xff)`, is recognized as bounded and is not flagged. The lint does not perform
control-flow analysis of preceding range checks.

## Why is this bad?

Solidity does **not** revert on narrowing casts; it silently keeps the lowest bits, which can
cause severe accounting bugs (e.g. amount overflows, wrong fees, broken invariants). Use a checked
cast helper such as OpenZeppelin's `SafeCast` whenever the source value is not provably bounded.

## Example

### Bad

```solidity
function setAmount(uint256 amount) external {
    smallAmount = uint128(amount); // silent truncation if amount >= 2**128
}
```

### Good

```solidity
function setAmount(uint256 amount) external {
    smallAmount = SafeCast.toUint128(amount);
}

// A mask that bounds the value to the target width is also recognized.
smallByte = uint8(amount & 0xff);
```
