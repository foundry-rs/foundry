# Immutables should use SCREAMING_SNAKE_CASE

**Severity**: `Info`
**ID**: `screaming-snake-case-immutable`

Flags `immutable` state variables whose names do not follow `SCREAMING_SNAKE_CASE`.

## What it does

Reports state variables declared `immutable` whose identifier deviates from
`SCREAMING_SNAKE_CASE`.

## Why is this bad?

The Solidity style guide recommends `SCREAMING_SNAKE_CASE` for `immutable` variables so they
visually align with `constant` ones and stand out from mutable state at call sites.

## Example

### Bad

```solidity
address immutable owner;
address immutable Owner;
```

### Good

```solidity
address immutable OWNER;
```
