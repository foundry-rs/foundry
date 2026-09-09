# High-level `keccak256` call

**Severity**: `Gas`
**ID**: `asm-keccak256`

Flags direct calls to the high-level `keccak256(...)` builtin for review as gas optimization
candidates.

## What it does

Reports `keccak256(arg)` when the call is the direct expression of a supported statement or
initializer. It does not estimate gas savings or emit an automatic rewrite.

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

## Notes

This is a `Gas`-severity lint and is **not** applied to test or script files.
