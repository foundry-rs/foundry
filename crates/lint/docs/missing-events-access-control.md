# Missing events access control

**Severity**: `Low`
**ID**: `missing-events-access-control`

Flags protected entry-point functions that update state used for access control without emitting an
event.

## What it does

This lint looks for mutable state variables that are:

- read by an access-control check involving `msg.sender` or `tx.origin`,
- written by a public or external state-mutating function with access control, and
- assigned from function input, directly or through local aliases/internal helpers, and
- changed by a function that does not emit any event directly or through an internal helper.

It intentionally skips constructors, unprotected setters, variables not used in authorization
checks, fixed writes, and functions that emit any event directly or through an internal helper.
Those limits keep the rule focused on Slither's low-severity `events-access` case while avoiding
common false positives.

## Why is this bad?

Off-chain monitors, users, and auditors often rely on events to track changes to owners, guardians,
roles, and other authority-bearing state. If a protected function silently changes access control,
critical permission updates are harder to review and investigate.

## Example

### Bad

```solidity
function transferOwnership(address newOwner) external onlyOwner {
    owner = newOwner;
}
```

### Good

```solidity
event OwnershipTransferred(address indexed oldOwner, address indexed newOwner);

function transferOwnership(address newOwner) external onlyOwner {
    address oldOwner = owner;
    owner = newOwner;
    emit OwnershipTransferred(oldOwner, newOwner);
}
```
