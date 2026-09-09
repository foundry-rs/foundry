# Inline assembly

**Severity**: `Info`
**ID**: `inline-assembly`

## What it does

Reports every inline assembly statement, including blocks marked `memory-safe`.

## Why restrict this?

Assembly bypasses Solidity's type and overflow checks and can corrupt memory. Prefer
high-level Solidity when it expresses the same operation. Assembly can still be appropriate
for measured gas savings or operations unavailable in Solidity; keep it small and document
its assumptions. Mark a block `memory-safe` only when it obeys Solidity's memory model.

## Example

```solidity
function chainId() external view returns (uint256 result) {
    assembly {
        result := chainid()
    }
}
```

Use instead:

```solidity
function chainId() external view returns (uint256) {
    return block.chainid;
}
```
