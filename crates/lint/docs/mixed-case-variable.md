# Mutable variable names should use mixedCase

**Severity**: `Info`
**ID**: `mixed-case-variable`

Flags mutable variable names (locals, parameters, mutable state) that do not follow `mixedCase`.

## What it does

Reports mutable variable identifiers that contain underscores, start with an uppercase letter,
or otherwise deviate from `mixedCase`.

`constant` and `immutable` state variables are not flagged by this lint — see
[`screaming-snake-case-const`](https://getfoundry.sh/forge/linting/screaming-snake-case-const) and
[`screaming-snake-case-immutable`](https://getfoundry.sh/forge/linting/screaming-snake-case-immutable).

## Why is this bad?

The Solidity style guide recommends `mixedCase` for mutable variables. Consistent style makes
code easier to scan and review.

## Example

### Bad

```solidity
uint256 public total_supply;
address Owner;
```

### Good

```solidity
uint256 public totalSupply;
address owner;
```
