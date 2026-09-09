# Constants should use `SCREAMING_SNAKE_CASE`

**Severity**: `Info`
**ID**: `screaming-snake-case-const`

## What it does

Reports state variables declared `constant` whose identifier is longer than one character and
deviates from `SCREAMING_SNAKE_CASE`. Leading and trailing underscores are preserved.

## Why restrict this?

The Solidity style guide recommends `SCREAMING_SNAKE_CASE` for constants so they stand out from
mutable state at call sites. Foundry recommends the same convention for immutables.

Keep an established naming convention or public constant getter name when changing it would
break compatibility or make the project less consistent.

## Example

```solidity
uint256 constant maxSupply = 1_000_000;
uint256 constant Max_Supply = 1_000_000;
```

Use instead:

```solidity
uint256 constant MAX_SUPPLY = 1_000_000;
```
