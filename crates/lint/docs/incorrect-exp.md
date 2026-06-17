# Incorrect exponentiation

**Severity**: `High`
**ID**: `incorrect-exp`

Flags `^` used between integer literals where `**` was almost certainly intended.

## What it does

Reports `a ^ b` when both `a` and `b` are non-hex integer literals. In Solidity `^` is bitwise xor, not exponentiation, so `10 ^ 18` evaluates to `24`, not `10 ** 18`. Hex literals are excluded, since xor of a bit pattern is a legitimate operation.

## Why is this bad?

Developers coming from languages where `^` means exponentiation (Python, many calculators) write `10 ^ 18` expecting `10 ** 18`. The contract compiles and silently uses the wrong constant, which can corrupt amounts, decimals, or limits.

## Example

### Bad

```solidity
uint256 constant WAD = 10 ^ 18; // evaluates to 24, not 1e18
```

### Good

```solidity
uint256 constant WAD = 10 ** 18;
```
