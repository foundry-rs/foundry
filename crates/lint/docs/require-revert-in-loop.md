# `require` or `revert` inside a loop

**Severity**: `Low`
**ID**: `require-revert-in-loop`

Flags `require` calls and `revert` statements inside loops because one invalid item can abort the
entire batch.

## What it does

Reports Solidity `require`/`revert`, revert statements, and Yul `revert` inside loops. The analysis
also follows modifiers and internal helper calls reached from a loop.

## Why restrict this?

A single invalid item can revert the entire loop, which can make batched operations unusable when
one element fails validation.

Atomic batches may intentionally require every item to succeed. Skip invalid items only when
partial processing is part of the API's intended behavior; otherwise keep the revert and suppress
the lint after review.

## Example

```solidity
contract Batch {
    function process(uint256[] calldata values) external {
        for (uint256 i; i < values.length; ++i) {
            require(values[i] != 0, "zero");
        }
    }
}
```

Use instead:

When the batch permits partial processing:

```solidity
contract Batch {
    function process(uint256[] calldata values) external {
        for (uint256 i; i < values.length; ++i) {
            if (values[i] == 0) continue;
            // Process valid entries.
        }
    }
}
```
