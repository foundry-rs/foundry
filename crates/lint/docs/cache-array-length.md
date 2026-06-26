# Array length not cached

**Severity**: `Gas`
**ID**: `cache-array-length`

Flags `for` loop conditions that read a storage dynamic array's `.length` on every iteration
instead of comparing against a cached local length.

## What it does

Reports simple comparison expressions in `for` loop conditions when either side reads `.length`
from a state dynamic array, such as `i < values.length` or `values.length > i`.

The lint does not report loops that already compare against a local cached length variable.
It also skips loops that mutate an array length in the loop body, such as calling `push()` or
`pop()`, because aliases can make caching the length change the loop semantics.

Fixed-size arrays are excluded because their length is a compile-time constant instead of a repeated
dynamic length lookup. This lint currently checks `for` loops.

## Why is this bad?

Reading `.length` in the loop condition repeats the storage length lookup for every iteration.
Caching the length once before entering the loop avoids repeated storage reads and can reduce gas
for hot loops.

## Example

### Bad

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

### Good

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

## Notes

This is a `Gas`-severity lint and is **not** applied to test or script files.
