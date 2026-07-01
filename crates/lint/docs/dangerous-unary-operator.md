# Dangerous unary operator

**Severity**: `Med`
**ID**: `dangerous-unary-operator`

Flags an assignment whose `=` is fused to a unary operator (`=-`, `=~`), which parses as a plain assignment of a unary expression rather than the compound assignment it resembles.

## What it does

Reports `x =- y` and `x =~ y` when the source writes `=` directly against the unary operator, with no space. `x =- 1` lexes as `x` `=` `-` `1` and parses as `x = -1`, identical to the intentional spaced form, but it is a common typo for the compound `x -= 1`. The spaced forms (`x = -1`, `x = ~y`) and the real compound operators (`x -= 1`) are left alone. This mirrors Slither's `dangerous-unary-expression`.

`=+` is not reported: unary `+` was removed in Solidity 0.5.0, so `x =+ 1` is a parse error and never reaches the linter.

## Why is this bad?

`x =- 1` silently assigns `-1` instead of decrementing `x`. Developers reaching for `-=` can transpose the operator into `=-`, and because the code compiles and runs, the wrong value is used with no warning.

## Example

### Bad

```solidity
x =- 1; // parses as `x = -1`, not `x -= 1`
```

### Good

```solidity
x -= 1;
```
