# Require or revert inside a loop

**Severity**: `Low`
**ID**: `require-revert-in-loop`

Flags `require` calls and `revert` statements inside loops because one invalid item can abort the
entire batch.

## What it does

Reports Solidity `require`/`revert`, revert statements, and Yul `revert` inside loops. The analysis
also follows modifiers and internal helper calls reached from a loop.

## Why is this bad?

A single invalid item can revert the entire loop, which can make batched operations unusable when
one element fails validation.

## Example

### Bad

```solidity
contract Batch {
    function process(uint256[] calldata values) external {
        for (uint256 i; i < values.length; ++i) {
            require(values[i] != 0, "zero");
        }
    }
}
```

### Good

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
