# Misuse of a boolean constant

**Severity**: `Med`
**ID**: `boolean-cst`

Flags expressions where a boolean constant (`true`/`false`) is used as a control-flow condition
or operand of a boolean operator, which usually indicates dead code or a leftover debug toggle.

## What it does

Reports `if (true)`, `if (false)`, `while (true)` outside of intentional infinite loops, and
boolean operators (`&&`, `||`) where one side is a literal `true`/`false`.

## Why is this bad?

A literal boolean as a condition makes the surrounding branch dead, hides logic errors, or
preserves a forgotten debug shortcut that bypasses real checks.

## Example

### Bad

```solidity
if (true) { // always taken
    doSomething();
}
require(condition && true, "unreachable"); // 'true' is redundant
```

### Good

```solidity
if (condition) {
    doSomething();
}
require(condition, "...");
```
