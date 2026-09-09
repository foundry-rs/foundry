# Missing events access control

**Severity**: `Low`
**ID**: `missing-events-access-control`

## What it does

Flags protected public or external functions that change ownership, roles, or other
state used in authorization checks without a related event containing the changed value
or key.

Constructors, unprotected setters, and fixed-value assignments are excluded, except
clearing the authority used to authorize the update.

## Why is this bad?

Off-chain monitors, users, and auditors often rely on events to track changes to owners, guardians,
roles, and other authority-bearing state. If a protected function silently changes access control,
critical permission updates are harder to review and investigate.

## Example

```solidity
function transferOwnership(address newOwner) external onlyOwner {
    owner = newOwner;
}
```

Use instead:

```solidity
event OwnershipTransferred(address indexed oldOwner, address indexed newOwner);

function transferOwnership(address newOwner) external onlyOwner {
    address oldOwner = owner;
    owner = newOwner;
    emit OwnershipTransferred(oldOwner, newOwner);
}
```
