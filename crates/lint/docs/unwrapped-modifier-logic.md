# Unwrapped modifier logic

**Severity**: `CodeSize`
**ID**: `unwrapped-modifier-logic`

Flags modifiers whose body contains non-trivial logic that should be moved into a helper function
to reduce contract code size.

## What it does

Reports modifiers whose body contains statements other than a single placeholder, simple builtin
calls (`require`/`assert`), or a single library function call. Modifiers that use inline assembly
are exempted.

## Why is this bad?

Solidity inlines a modifier's body at every call site, so any non-trivial logic is duplicated
across all functions that use the modifier. Wrapping the logic in an internal function and calling
it from the modifier keeps the bytecode small while preserving behavior.

## Example

### Bad

```solidity
modifier onlyAuth() {
    if (!auth[msg.sender]) revert NotAuth();
    bytes32 nonce = keccak256(abi.encodePacked(msg.sender, block.number));
    seenNonce[nonce] = true;
    _;
}
```

### Good

```solidity
modifier onlyAuth() {
    _checkAuth();
    _;
}

function _checkAuth() internal {
    if (!auth[msg.sender]) revert NotAuth();
    bytes32 nonce = keccak256(abi.encodePacked(msg.sender, block.number));
    seenNonce[nonce] = true;
}
```

## Notes

This is a `CodeSize`-severity lint and is **not** applied to test or script files.
