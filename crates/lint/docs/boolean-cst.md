# Misuse of a boolean constant

**Severity**: `Med`
**ID**: `boolean-cst`

Flags expressions where a boolean constant (`true`/`false`) is used as a control-flow condition
or operand of a boolean operator, which usually indicates dead code or a leftover debug toggle.

## What it does

Reports literal boolean conditions in `if`, `for`, and `do while`, `while (false)`, and
boolean operators (`&&`, `||`) where one side is a literal `true`/`false`.
The idiomatic infinite loop `while (true)` is exempt.

## Why is this bad?

A literal boolean as a condition makes the surrounding branch dead, hides logic errors, or
preserves a forgotten debug shortcut that bypasses real checks.

## Example

```solidity
if (true) { // always taken
    doSomething();
}
require(condition && true, "unreachable"); // 'true' is redundant
```

Use instead:

```solidity
if (condition) {
    doSomething();
}
require(condition, "...");
```
