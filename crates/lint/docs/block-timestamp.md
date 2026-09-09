# Use of block.timestamp in comparisons

**Severity**: `Low`
**ID**: `block-timestamp`

Flags use of `block.timestamp` as an operand of a comparison, where its value can be slightly
manipulated by the block proposer.

## What it does

Reports any comparison expression (`<`, `<=`, `>`, `>=`, `==`, `!=`) that directly or
transitively reads `block.timestamp`.

## Why is this bad?

Block proposers can adjust `block.timestamp` within a small window (a few seconds). This is
usually harmless, but for short-window logic — auctions ending, randomness, time-locked
withdrawals — a few seconds of manipulation can be enough for an attacker to capture value.

Using `block.timestamp` for general scheduling (hours/days) is fine; what's risky is fine-grained
timing and treating timestamps as a source of randomness.

## Example

### Bad

```solidity
function settle() external {
    require(block.timestamp >= auctionEnd, "auction ongoing");
    // ...
}
```

### Good

```solidity
// Prefer block numbers for tight windows, or accept a clearly large grace period.
require(block.number >= endBlock, "auction ongoing");
```

## Notes

This lint is intentionally conservative: not every flagged comparison is exploitable. Review
each occurrence in context.
