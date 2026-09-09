# Dangerous unary operator

**Severity**: `Med`
**ID**: `dangerous-unary-operator`

## What it does

Reports `x =- y` and `x =~ y`, where `=` is written directly beside a unary operator.
These are assignments, not compound operations: `x =- 1` means `x = -1`, not `x -= 1`.
The intentional spaced forms (`x = -1`, `x = ~y`) and compound operators (`x -= 1`)
are not flagged.

Unary `+` is invalid in Solidity 0.5.0 and later, so `=+` is a compiler error.

## Why is this bad?

`x =- 1` silently assigns `-1` instead of decrementing `x`. Developers reaching for `-=` can transpose the operator into `=-`, and because the code compiles and runs, the wrong value is used with no warning.

## Example

```solidity
x =- 1; // parses as `x = -1`, not `x -= 1`
```

Use instead:

```solidity
x -= 1;
```
