# require-revert-in-loop

## Description

Detects `require` calls and `revert` statements inside loops, including checks reached through
internal helper calls, modifier placeholders, and inline assembly `revert`.

## Why is this bad?

A single invalid item can revert the entire loop, which can make batched operations unusable when
one element fails validation.

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

## Recommended

Validate inputs before the loop, skip invalid entries explicitly, or split work into smaller calls
when one failing element should not revert the whole batch.
