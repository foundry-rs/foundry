# Incorrect Modifier

**Severity**: `Low`
**ID**: `incorrect-modifier`

## What it does

Flags modifiers that can finish successfully without reaching the `_` placeholder.
A path that reverts before `_` is allowed, but calling a function that might revert
does not by itself prevent the modifier from skipping the body.

## Why is this bad?

A modifier that falls through before `_` silently skips the function body. This can make calls look
successful even though the protected action never ran.

## Example

```solidity
modifier onlyWhenEnabled() {
    if (enabled) {
        _;
    }
}
```

Use instead:

```solidity
modifier onlyWhenEnabled() {
    if (!enabled) {
        revert Disabled();
    }
    _;
}
```
