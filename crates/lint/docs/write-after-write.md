# Write after write

**Severity**: `Gas`
**ID**: `write-after-write`

Flags storage variables that are written to consecutively without the first value ever being read.
The first write is dead code; it pays the SSTORE cost but its value is immediately discarded when
the second write overwrites it.

## What it does

Within a flat sequence of statements (no intervening branches or loops), reports the first
assignment to a simple state variable when that same variable is written a second time before being
read. Only plain `=` assignments and `delete` expressions on bare state variable identifiers are
checked; compound assignments (`+=`, `|=`, etc.) and index/member writes (`mapping[k]`,
`struct.field`) are conservatively excluded to avoid false positives.

## Why is this bad?

Every SSTORE costs at least 2,900 gas (warm slot) or 20,000 gas (cold slot). Writing a value to
storage and then immediately overwriting it wastes that gas with no observable effect; only the
final write matters.

## Example

### Bad

```solidity
contract C {
    uint256 public x;

    function bad(uint256 v) external {
        x = 0;   // write-after-write: this value is never read
        x = v;   // second write overwrites the first
    }
}
```

### Good

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

    // Compound assignments read before writing, not flagged.
    function goodCompound() external {
        x = 1;
        x += 1;
    }
}
```
