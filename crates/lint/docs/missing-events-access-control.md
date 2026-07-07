# Missing events access control

**Severity**: `Low`
**ID**: `missing-events-access-control`

Flags protected entry-point functions that update state used for access control without emitting an
event.

## What it does

This lint looks for mutable state variables that are:

- read by an access-control check involving `msg.sender` or `tx.origin`,
- written by a public or external state-mutating function with access control, and
- assigned from function input, `msg.sender`, another access-control state variable, or a keyed
  mapping write, directly or through local aliases/internal helpers, and
- changed without a related event that includes the same value or key source.

It intentionally skips constructors, unprotected setters, variables not used in authorization
checks, unrelated events, and fixed writes other than clearing the state variable currently used by
the access guard. Those limits keep the rule focused on Slither's low-severity `events-access` case
while avoiding common false positives.

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
