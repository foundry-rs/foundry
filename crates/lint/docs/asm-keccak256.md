# High-level `keccak256` call

**Severity**: `Gas`
**ID**: `asm-keccak256`

## What it does

Reports direct `keccak256(...)` calls in statements and initializers for gas review.

## Why is this bad?

Hashing freshly encoded arguments can allocate memory and copy data. A carefully written assembly
block can avoid some of that work. Savings depend on the input layout and compiler settings;
measure them before replacing high-level code, and preserve the exact bytes being hashed.

## Example

```solidity
function hashPair(bytes32 a, bytes32 b) internal pure returns (bytes32) {
    return keccak256(abi.encodePacked(a, b));
}
```

Use instead:

```solidity
function hashPair(bytes32 a, bytes32 b) internal pure returns (bytes32 h) {
    assembly ("memory-safe") {
        mstore(0x00, a)
        mstore(0x20, b)
        h := keccak256(0x00, 0x40)
    }
}
```
