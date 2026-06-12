# State variable could be constant

**Severity**: `Gas`
**ID**: `could-be-constant`

Flags state variables that have a compile-time-constant inline initializer and are never written
anywhere — making them eligible to be declared `constant`.

## What it does

Reports each non-`constant`, non-`immutable` state variable when **all** of the following hold:

- Its type is constant-compatible: any elementary type (value types, `string`, `bytes`) or a
  contract type.
- It has an inline initializer composed only of literals, type casts (`address(...)`,
  `uint160(...)`, contract/interface casts like `IToken(...)`), `type(T).{min,max,interfaceId}`,
  allowed pure builtin calls (`keccak256`, `sha256`, `ripemd160`, `ecrecover`, `addmod`,
  `mulmod`), and references to other `constant` variables.
- It is never written by the constructor body or by any other function.

## Why is this bad?

`constant` state variables are inlined directly into the deployed bytecode rather than read from
storage, eliminating `SLOAD` costs on every access. Declaring such variables `constant` also
expresses intent and prevents future writes.

## Example

### Bad

```solidity
contract C {
    uint256 LIMIT = 100;
    bytes32 SALT = keccak256("foundry");
}
```

### Good

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
