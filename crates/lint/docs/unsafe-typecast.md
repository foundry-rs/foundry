# Unsafe typecast

**Severity**: `Med`
**ID**: `unsafe-typecast`

Flags explicit numeric typecasts that can silently truncate or alter the value.

## What it does

Reports casts where the source value's type is wider than the target type
(e.g. `uint256 → uint128`, `int256 → uint128`), unless the cast is preceded by a check that
guarantees the value fits in the target.

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
    require(amount <= type(uint128).max, "overflow");
    smallAmount = uint128(amount);
}

// or
smallAmount = SafeCast.toUint128(amount);
```
