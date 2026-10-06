# Missing events access control

**Severity**: `Low`
**ID**: `missing-events-access-control`

## What it does

Flags protected public or external functions that change ownership, roles, or other
state used in authorization checks without a related event containing the changed value
or key.

Constructors, unprotected setters, and fixed-value assignments are excluded, except
clearing the authority used to authorize the update.

A related event can appear before or after the write within the same straight-line path,
including in an internal helper. An event confined to a conditional branch, a
short-circuit operand, a loop, or a try/catch clause does not cover a write on a
path that can skip that event. Writes in a `for` loop's update expression also
require a related event.

A helper that can return without emitting does not provide event coverage for a
write in its caller. Rebinding a storage pointer alone is not a state change.

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
