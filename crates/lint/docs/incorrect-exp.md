# Incorrect exponentiation

**Severity**: `High`
**ID**: `incorrect-exp`

Flags `^` used between integer literals where `**` was almost certainly intended.

## What it does

Reports `a ^ b` when the base `a` is `2` or `10` and both operands are decimal integer literals (looking through casts such as `uint256(10)`). In Solidity `^` is bitwise xor, not exponentiation, so `10 ^ 18` evaluates to `24`, not `10 ** 18`.

The base is restricted to the values people actually write as powers (`2` for bit widths, `10` for decimals), and operands written in hex are left alone. This mirrors GCC's and Clang's `-Wxor-used-as-pow`, so legitimate decimal bitmask xors such as `255 ^ 128` are not flagged.

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
