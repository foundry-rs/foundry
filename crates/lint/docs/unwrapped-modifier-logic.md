# Unwrapped modifier logic

**Severity**: `CodeSize`
**ID**: `unwrapped-modifier-logic`

## What it does

Reports modifier logic that can be extracted around a single top-level `_` placeholder,
including `require` and `assert` checks. A single ordinary function or library call on
either side is left inline. A side containing inline assembly is not extracted.

## Why is this bad?

Modifier logic can be duplicated across functions that use it. Extracting shared logic into an
internal helper can reduce that duplication, but the optimizer may inline the helper again.
Treat extraction as a code-size optimization candidate and measure the compiled output with the
project's compiler settings while preserving modifier behavior.

Suggested helper names can collide with existing declarations, and extraction can affect
virtual dispatch or reference aliasing. Review the replacement rather than applying it automatically.

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
