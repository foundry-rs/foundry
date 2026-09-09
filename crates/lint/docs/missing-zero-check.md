# Missing zero-address check

**Severity**: `Low`
**ID**: `missing-zero-check`

Flags entry-point functions and constructors where an `address` parameter flows into a state write
or value transfer without a zero-address guard.

## What it does

Performs a taint analysis from each `address` parameter of an externally callable, state-mutating
function (or constructor) and reports a parameter that reaches a sink (state write, `transfer`,
`call{value: ...}`, etc.) without first being compared against `address(0)` in an `if`/`require`/
`assert` predicate.

## Why is this bad?

Forgetting a zero-address check is a common source of value loss: tokens become permanently
unrecoverable, ownership is renounced unintentionally, or upgrades are bricked. Adding an explicit
guard is cheap and removes an entire class of operational mistakes.

## Example

### Bad

```solidity
function setOwner(address newOwner) external onlyOwner {
    owner = newOwner; // no zero-address check
}
```

### Good

```solidity
function setOwner(address newOwner) external onlyOwner {
    require(newOwner != address(0), "zero address");
    owner = newOwner;
}
```
