# Inline assembly

**Severity**: `Info`
**ID**: `inline-assembly`

Flags every `assembly { ... }` block. Inline assembly bypasses many of Solidity's safety
features (type checks, overflow checks, memory layout invariants) and is a common source of
high-impact bugs, so each occurrence should be reviewed deliberately.

## What it does

Reports every inline assembly statement, including blocks declared with the `"evmasm"` dialect
and/or the `("memory-safe")` flag. Blocks declared as memory-safe — either via the modern
`("memory-safe")` flag or the legacy `/// @solidity memory-safe-assembly` NatSpec marker — are
still reported, but with a softer message acknowledging the developer attestation: review
focuses on business logic and side effects rather than memory layout.

## Why is this bad?

Assembly skips Solidity's compile-time checks and many of its runtime guarantees. Mistakes
inside an `assembly` block can corrupt memory, break the free memory pointer, leak storage,
escalate privileges via `delegatecall`, or destroy the contract via `selfdestruct`. Even when
required for gas or features unavailable in high-level Solidity, assembly should be small,
documented, and reviewed.

## When inline assembly is reasonable

Some idioms are widely used and generally safe:

- Reading transaction/chain context: `chainid()`, `gas()`, `returndatasize()`.
- Probing code: `codesize()`, `extcodesize(addr)`, `extcodehash(addr)`.
- Reading the free memory pointer: `mload(0x40)`.
- Cheap hashing of a known memory layout, when paired with `("memory-safe")`.

If you must use assembly:

1. Keep the block minimal and well-commented.
2. Add the `("memory-safe")` flag when the block does not violate Solidity's memory model, so
   the optimizer (and reviewers) can rely on it. The legacy
   `/// @solidity memory-safe-assembly` NatSpec marker on the line directly above the block is
   also recognized for compatibility with older codebases.
3. Suppress the lint locally to mark the block as audited:
   ```solidity
   // forge-lint: disable-next-line(inline-assembly)
   assembly ("memory-safe") { /* reviewed: ... */ }
   ```

## Example

### Bad

```solidity
function rawCall(address target, bytes calldata data) external returns (bytes memory) {
    assembly {
        let ok := call(gas(), target, 0, add(data.offset, 0), data.length, 0, 0)
        // ...
    }
}
```

### Good

```solidity
function rawCall(address target, bytes calldata data) external returns (bytes memory result) {
    bool ok;
    (ok, result) = target.call(data);
    require(ok, "call failed");
}
```
