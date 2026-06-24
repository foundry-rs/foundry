# Unused state variable

**Severity**: `Gas`
**ID**: `unused-state-variables`

Flags state variables that are declared but never read or written anywhere in the contract or its
descendants.

## What it does

Reports each state variable that has no read or write site across the project.

## Why is this bad?

Unused state variables waste storage slots, inflate deployment cost, and are a strong signal of
dead or stale code that should be removed.

## Example

### Bad

```solidity
contract C {
    uint256 unused;       // never read or written
    uint256 public total; // used elsewhere
}
```

### Good

```solidity
contract C {
    uint256 public total;
}
```

## Notes

This is a `Gas`-severity lint and is **not** applied to test or script files.
