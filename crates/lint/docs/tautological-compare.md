# Tautological compare

**Severity**: `Med`
**ID**: `tautological-compare`

## What it does

Reports `a <op> a` where `a` is a side-effect-free expression (an identifier, member access, or indexing) and `<op>` is `<`, `<=`, `>`, `>=`, `==`, or `!=`. Such a comparison has a constant result. Comparisons whose sides could legitimately differ (for example involving a function call) are left untouched, as are comparisons on user-defined value types, whose operators are user-defined (`using {f as ==} for T`) and need not be constant.

## Why is this bad?

`x >= x` is always true and `x < x` is always false, regardless of `x`. It is almost always a typo for a comparison against a different operand, or leftover dead code that hides a real condition.

## Example

```solidity
require(balance >= balance); // always true; likely meant another operand
if (a[i] < a[i]) {           // always false; dead branch
    // ...
}
```

Use instead:

```solidity
require(balance >= amount);
if (a[i] < a[j]) {
    // ...
}
```
