# State variable could be immutable

**Severity**: `Gas`
**ID**: `could-be-immutable`

## What it does

Reports each non-`constant`, non-`immutable` state variable whose only writes occur in the
constructor (or in initialization at declaration time).

## Why is this bad?

`immutable` state variables are stored in the deployed bytecode rather than in storage, eliminating
an `SLOAD` per access and saving substantial gas across the contract's lifetime. Declaring such
variables `immutable` also expresses intent and prevents future writes.

## Example

```solidity
contract C {
    address owner;
    constructor() { owner = msg.sender; }
}
```

Use instead:

```solidity
contract C {
    address immutable OWNER;
    constructor() { OWNER = msg.sender; }
}
```
