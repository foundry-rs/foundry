# Boolean comparison to a constant

**Severity**: `Info`
**ID**: `boolean-equal`

Flags expressions of the form `x == true`, `x == false`, `x != true`, `x != false`, which can be
simplified.

## What it does

Reports any equality comparison between a boolean expression and a literal `true` or `false`.

## Why is this bad?

Comparing a boolean to a boolean literal is redundant and harms readability. Use the boolean
expression directly (or its negation).

## Example

### Bad

```solidity
if (paused == true) revert();
if (paused == false) doSomething();
require(ok != false, "fail");
```

### Good

```solidity
if (paused) revert();
if (!paused) doSomething();
require(ok, "fail");
```
