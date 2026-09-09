# Boolean comparison to a constant

**Severity**: `Info`
**ID**: `boolean-equal`

## What it does

Reports any equality comparison between a boolean expression and a literal `true` or `false`.

## Why restrict this?

Comparing a boolean to a boolean literal is redundant and harms readability. Use the boolean
expression directly (or its negation).

## Example

```solidity
if (paused == true) revert();
if (paused == false) doSomething();
require(ok != false, "fail");
```

Use instead:

```solidity
if (paused) revert();
if (!paused) doSomething();
require(ok, "fail");
```
