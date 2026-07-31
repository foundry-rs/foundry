# Mutable variable names should use mixedCase

**Severity**: `Info`
**ID**: `mixed-case-variable`

Flags mutable variable names (locals, parameters, mutable state) that do not follow `mixedCase`.

## What it does

Reports mutable variable identifiers that contain embedded underscores, start with an uppercase
letter, or otherwise deviate from `mixedCase`. Leading and trailing underscores are preserved, and
single-character names are not checked.

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

## Configuration

Set `mixed_case_exceptions` under `[lint.lint_specific]` in `foundry.toml` to replace the default
list of allowed uppercase patterns (`ERC`, `URI`, `ID`, `URL`, `API`, `JSON`, `XML`, `HTML`, `HTTP`,
and `HTTPS`):

```toml
[lint.lint_specific]
mixed_case_exceptions = ["ERC", "URI", "NFT"]
```
