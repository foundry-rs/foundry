# Unwrapped modifier logic

**Severity**: `CodeSize`
**ID**: `unwrapped-modifier-logic`

Flags modifiers whose body contains non-trivial logic that should be moved into a helper function
to reduce contract code size.

## What it does

Reports modifiers containing logic beyond a placeholder, simple `require` or `assert`
checks, or a single library call. Assembly blocks are excluded from suggested extraction.

## Why is this bad?

Modifier logic can be duplicated across functions that use it. Extracting shared logic into an
internal helper can reduce that duplication, but the optimizer may inline the helper again.
Treat extraction as a code-size optimization candidate and measure the compiled output with the
project's compiler settings while preserving modifier behavior.

## Example

```solidity
modifier onlyAuth() {
    if (!auth[msg.sender]) revert NotAuth();
    bytes32 nonce = keccak256(abi.encodePacked(msg.sender, block.number));
    seenNonce[nonce] = true;
    _;
}
```

Use instead:

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
