# Unused state variable

**Severity**: `Gas`
**ID**: `unused-state-variables`

## What it does

Reports each state variable that has no read or write site across the project.

## Why is this bad?

Unused state variables occupy storage layout positions and can indicate dead or stale code.
An untouched slot does not itself incur an `SSTORE` charge. Before removing a variable from an
upgradeable contract, preserve the storage layout expected by existing deployments.

## Example

```solidity
contract C {
    uint256 unused;       // never read or written
    uint256 public total; // used elsewhere
}
```

Use instead:

```solidity
contract C {
    uint256 public total;
}
```
