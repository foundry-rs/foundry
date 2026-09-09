# Array length not cached

**Severity**: `Gas`
**ID**: `cache-array-length`

## What it does

Reports comparison expressions in `for` loop conditions when either side reads `.length` from a
state dynamic array, such as `i < values.length` or `values.length > i`, including comparisons
nested inside `&&` / `||` conditions.

Loops that change an array's length with operations such as `push()` or `pop()` are
excluded because caching the length can change which elements are visited.

## Why is this bad?

Reading `.length` in the loop condition repeats the storage length lookup for every iteration.
Caching the length once before entering the loop avoids repeated storage reads and can reduce gas
for hot loops.

## Example

```solidity
contract C {
    uint256[] values;

    function sum() external view returns (uint256 total) {
        for (uint256 i = 0; i < values.length; ++i) {
            total += values[i];
        }
    }
}
```

Use instead:

```solidity
contract C {
    uint256[] values;

    function sum() external view returns (uint256 total) {
        uint256 length = values.length;
        for (uint256 i = 0; i < length; ++i) {
            total += values[i];
        }
    }
}
```
