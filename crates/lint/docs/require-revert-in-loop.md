# `require` or `revert` inside a loop

**Severity**: `Low`
**ID**: `require-revert-in-loop`

## What it does

Reports `require` calls and Solidity or Yul `revert` operations inside loops.
This includes operations reached through modifiers and internal helpers called from a loop.

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
