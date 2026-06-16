# Tautological compare

**Severity**: `Med`
**ID**: `tautological-compare`

Flags a relational or equality comparison whose two sides are the same expression.

## What it does

Reports `a <op> a` where `a` is a side-effect-free expression (an identifier, member access, or indexing) and `<op>` is `<`, `<=`, `>`, `>=`, `==`, or `!=`. Such a comparison has a constant result. Comparisons whose sides could legitimately differ (for example involving a function call) are left untouched.

## Why is this bad?

`x >= x` is always true and `x < x` is always false, regardless of `x`. It is almost always a typo for a comparison against a different operand, or leftover dead code that hides a real condition.

## Example

### Bad

```solidity
require(balance >= balance); // always true; likely meant another operand
if (a[i] < a[i]) {           // always false; dead branch
    // ...
}
```

### Good

```solidity
require(balance >= amount);
if (a[i] < a[j]) {
    // ...
}
```
