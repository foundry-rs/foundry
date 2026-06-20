# State variable could be immutable

**Severity**: `Gas`
**ID**: `could-be-immutable`

Flags state variables that are assigned only in the constructor and never written to afterward —
making them eligible to be declared `immutable`.

## What it does

Reports each non-`constant`, non-`immutable` state variable whose only writes occur in the
constructor (or in initialization at declaration time).

## Why is this bad?

`immutable` state variables are stored in the deployed bytecode rather than in storage, eliminating
an `SLOAD` per access and saving substantial gas across the contract's lifetime. Declaring such
variables `immutable` also expresses intent and prevents future writes.

## Example

### Bad

```solidity
contract C {
    address owner;
    constructor() { owner = msg.sender; }
}
```

### Good

```solidity
contract C {
    address immutable OWNER;
    constructor() { OWNER = msg.sender; }
}
```

## Notes

This is a `Gas`-severity lint and is **not** applied to test or script files.
