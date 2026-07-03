# Constants should use SCREAMING_SNAKE_CASE

**Severity**: `Info`
**ID**: `screaming-snake-case-const`

Flags `constant` state variables whose names do not follow `SCREAMING_SNAKE_CASE`.

## What it does

Reports state variables declared `constant` whose identifier deviates from `SCREAMING_SNAKE_CASE`.

## Why is this bad?

The Solidity style guide recommends `SCREAMING_SNAKE_CASE` for constants so they stand out from
mutable state and immutables at call sites.

## Example

### Bad

```solidity
uint256 constant maxSupply = 1_000_000;
uint256 constant Max_Supply = 1_000_000;
```

### Good

```solidity
uint256 constant MAX_SUPPLY = 1_000_000;
```
