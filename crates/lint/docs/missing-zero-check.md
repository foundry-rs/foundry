# Missing zero-address check

**Severity**: `Low`
**ID**: `missing-zero-check`

## What it does

Reports `address` parameters used in a state write or value transfer by an externally
callable state-mutating function or constructor without a check against `address(0)`.

## Why is this bad?

Forgetting a zero-address check is a common source of value loss: tokens become permanently
unrecoverable, ownership is renounced unintentionally, or upgrades are bricked. Adding an explicit
guard is cheap and removes an entire class of operational mistakes.

## Example

```solidity
function setOwner(address newOwner) external onlyOwner {
    owner = newOwner; // no zero-address check
}
```

Use instead:

```solidity
function setOwner(address newOwner) external onlyOwner {
    require(newOwner != address(0), "zero address");
    owner = newOwner;
}
```
