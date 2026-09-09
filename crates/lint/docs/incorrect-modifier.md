# Incorrect Modifier

**Severity**: `Low`
**ID**: `incorrect-modifier`

Reports modifiers that can finish without executing the modified function body or reverting.

## What it does

Flags Solidity modifiers where at least one path can complete before reaching the `_` placeholder.
Paths that revert before `_` are not flagged.

Only a path that provably reverts or otherwise halts is treated as safe: an explicit `revert`
statement or `revert(...)` builtin call, and the Yul halting builtins (`revert`/`invalid` to fail,
and `return`/`stop`/`selfdestruct`, which halt *successfully* without running the function body and
are therefore flagged). A regular function call that *might* revert (e.g. `require(...)`, an
internal helper, or an external call) does not count, so a path that performs such a call and then
returns without reaching `_` is still flagged. This is intentionally stricter than some other
tools.

## Why is this bad?

A modifier that falls through before `_` silently skips the function body. This can make calls look
successful even though the protected action never ran.

## Example

### Bad

```solidity
modifier onlyWhenEnabled() {
    if (enabled) {
        _;
    }
}
```

### Good

```solidity
modifier onlyWhenEnabled() {
    if (!enabled) {
        revert Disabled();
    }
    _;
}
```
