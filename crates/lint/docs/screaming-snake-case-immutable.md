# Immutables should use `SCREAMING_SNAKE_CASE`

**Severity**: `Info`
**ID**: `screaming-snake-case-immutable`

## What it does

Reports state variables declared `immutable` whose identifier deviates from
`SCREAMING_SNAKE_CASE`. Single-character names are not checked, and leading and trailing
underscores are preserved.

## Why restrict this?

The Solidity style guide recommends `SCREAMING_SNAKE_CASE` for `immutable` variables so they
visually align with `constant` ones and stand out from mutable state at call sites.

Some projects distinguish immutables from constants through another naming convention. Keep that
convention, or an established public getter name, when a rename would be disruptive.

## Example

```solidity
address immutable owner;
address immutable Owner;
```

Use instead:

```solidity
address immutable OWNER;
```
