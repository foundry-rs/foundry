# State variable could be constant

**Severity**: `Gas`
**ID**: `could-be-constant`

Flags state variables that have a compile-time-constant inline initializer and are never written
anywhere — making them eligible to be declared `constant`.

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

## Notes

This lint requires an inline compile-time-constant initializer. Variables without an initializer
(`uint256 x;`) are not flagged, since converting them to `constant` requires choosing a value, not
just adding a keyword. This is a `Gas`-severity lint and is **not** applied to test or script
files.
