# Inefficient keccak256 call

**Severity**: `Gas`
**ID**: `asm-keccak256`

Flags calls to the high-level `keccak256(...)` builtin that can be cheaply rewritten with inline
assembly.

## What it does

Reports `keccak256(arg)` calls and (when possible) emits a fix suggestion that uses inline
assembly to compute the hash directly, avoiding the overhead of the high-level call.

## Why is this bad?

The high-level `keccak256` call performs additional memory management and ABI encoding compared
to a direct `keccak256(ptr, len)` opcode invocation. In hot paths the difference is visible in
gas reports.

## Example

### Bad

```solidity
bytes32 h = keccak256(abi.encodePacked(a, b));
```

### Good

```solidity
bytes32 h;
assembly ("memory-safe") {
    let m := mload(0x40)
    mstore(m, a)
    mstore(add(m, 0x20), b)
    h := keccak256(m, 0x40)
}
```

## Notes

This is a `Gas`-severity lint and is **not** applied to test or script files.
