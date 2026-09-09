# Incorrect exponentiation

**Severity**: `High`
**ID**: `incorrect-exp`

## What it does

Reports `a ^ b` when both operands are decimal integer literals and `a` is `2` or `10`.
In Solidity, `^` is bitwise XOR, so `10 ^ 18` evaluates to `24`, not `10 ** 18`.
Hexadecimal and scientific-notation operands are excluded.

## Why is this bad?

Developers familiar with mathematical notation or calculators where `^` means exponentiation may
write `10 ^ 18` expecting `10 ** 18`. The contract compiles and silently uses the wrong constant,
which can corrupt amounts, decimals, or limits.

## Example

```solidity
uint256 constant WAD = 10 ^ 18; // evaluates to 24, not 1e18
```

Use instead:

```solidity
uint256 constant WAD = 10 ** 18;
```
