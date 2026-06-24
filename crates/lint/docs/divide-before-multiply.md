# Divide before multiply

**Severity**: `Med`
**ID**: `divide-before-multiply`

Flags arithmetic expressions where division is performed before multiplication, which can cause
unintended precision loss in integer arithmetic.

## What it does

Warns on expressions of the form `(a / b) * c` (or equivalent shapes), where the integer division
truncates before the result is multiplied.

## Why is this bad?

Solidity's integer division truncates toward zero. Performing `(a / b) * c` discards the remainder
of `a / b` before scaling, while `(a * c) / b` preserves precision. This pattern frequently
manifests as fee/share/yield miscalculations.

## Example

### Bad

```solidity
uint256 share = (amount / total) * weight; // truncates first, then scales
```

### Good

```solidity
uint256 share = (amount * weight) / total; // preserves precision
```
