# Use of `block.timestamp` in comparisons

**Severity**: `Low`
**ID**: `block-timestamp`

## What it does

Reports comparison expressions (`<`, `<=`, `>`, `>=`, `==`, `!=`) involving `block.timestamp`.

## Why is this bad?

Timestamp rules depend on the chain's consensus protocol; there is no universal number of seconds
by which a proposer can adjust a timestamp. Transaction ordering and delayed inclusion can also
affect which side of a deadline an operation reaches. Review timing-sensitive logic against the
target chain's guarantees and avoid using timestamps as unpredictable randomness.

Ordinary scheduling may intentionally use `block.timestamp`. Block numbers are not a universal
substitute for elapsed time. When a deadline comparison is intended, document that assumption and
suppress this conservative lint locally.

## Example

```solidity
function settle() external {
    require(block.timestamp >= auctionEnd, "auction ongoing");
    // ...
}
```

Use instead:

```solidity
function settle() external {
    // This auction intentionally permits settlement any time after its deadline.
    // forge-lint: disable-next-line(block-timestamp)
    require(block.timestamp >= auctionEnd, "auction ongoing");
    // ...
}
```
