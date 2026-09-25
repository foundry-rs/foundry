# Write after write

**Severity**: `Gas`
**ID**: `write-after-write`

## What it does

Reports assignments to state variables whose values are overwritten before being read.
Compound assignments read before writing and are reported when their result is overwritten.
Individual mapping entries, array elements, and struct fields are excluded.

## Why is this bad?

Writing a value to storage and then immediately overwriting it can waste gas when the compiler
does not eliminate the first write. The cost depends on slot access history and the values
involved; it is not a fixed amount per write. Remove the first assignment only when
its evaluation has no required side effects or revert checks, including arithmetic overflow checks.

## Example

```solidity
contract C {
    uint256 public x;

    function bad(uint256 v) external {
        x = 0;   // write-after-write: this value is never read
        x = v;   // second write overwrites the first
    }
}
```

Use instead:

```solidity
contract C {
    uint256 public x;

    // Write once with the final value directly.
    function good(uint256 v) external {
        x = v;
    }

    // Reading between writes is fine.
    function goodRead(uint256 v) external returns (uint256 prev) {
        x = 1;
        prev = x; // x is read here
        x = v;
    }

    // The compound assignment reads the earlier write.
    function goodCompound() external {
        x = 1;
        x += 1;
    }
}
```
