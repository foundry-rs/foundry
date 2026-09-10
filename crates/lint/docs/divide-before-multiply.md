# Divide before multiply

**Severity**: `Med`
**ID**: `divide-before-multiply`

## What it does

Warns on expressions of the form `(a / b) * c` (or equivalent shapes), where the integer division
truncates before the result is multiplied.

## Why is this bad?

Solidity's integer division truncates toward zero. Performing `(a / b) * c` discards the remainder
of `a / b` before scaling, while `(a * c) / b` preserves precision. This pattern frequently
manifests as fee/share/yield miscalculations.

Multiplying first can overflow even when the final result fits. Use this rewrite only when the
product fits the integer type; otherwise use a checked full-precision multiplication/division
helper. Decide explicitly which rounding behavior the calculation requires.

## Example

```solidity
uint256 share = (amount / total) * weight; // truncates first, then scales
```

Use instead:

```solidity
uint256 share = (amount * weight) / total; // preserves precision
```
