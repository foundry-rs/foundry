# Incorrect exponentiation

**Severity**: `High`
**ID**: `incorrect-exp`

Flags `^` used between integer literals where `**` was almost certainly intended.

## What it does

Reports `a ^ b` when the base `a` is `2` or `10` and both operands are plain decimal integer literals, looking through integer casts such as `uint256(10)`. In Solidity `^` is bitwise xor, not exponentiation, so `10 ^ 18` evaluates to `24`, not `10 ** 18`.

The base is restricted to the values people write as powers (`2` for bit widths, `10` for decimals). Operands written in hex (`0xff`), in scientific notation (`1e1`), or behind a non-integer cast (`bytes32(...)`) are left alone: the lint prefers a false negative to a false positive that would annoy developers. This mirrors GCC's and Clang's `-Wxor-used-as-pow`.

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
