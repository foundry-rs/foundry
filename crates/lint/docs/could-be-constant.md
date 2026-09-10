# State variable could be constant

**Severity**: `Gas`
**ID**: `could-be-constant`

## What it does

Reports non-`constant`, non-`immutable` state variables with a compile-time-constant
initializer and no later assignments, when their type permits `constant`.

## Why is this bad?

`constant` state variables are inlined directly into the deployed bytecode rather than read from
storage, eliminating `SLOAD` costs on every access. Declaring such variables `constant` also
expresses intent and prevents future writes.

## Example

```solidity
contract C {
    uint256 LIMIT = 100;
    bytes32 SALT = keccak256("foundry");
}
```

Use instead:

```solidity
contract C {
    uint256 constant LIMIT = 100;
    bytes32 constant SALT = keccak256("foundry");
}
```
