# Write after write

**Severity**: `Gas`
**ID**: `write-after-write`

Flags storage variables that are written to consecutively without the first value ever being read.
The first write is dead code; it pays the SSTORE cost but its value is immediately discarded when
the second write overwrites it.

## What it does

Reports the first assignment to a state variable when that same variable is written a second time
before being read. Detection covers:

- Plain `=` assignments and `delete` on bare state variable identifiers
- Tuple/destructuring assignments: `(x, y) = (1, 2)` tracks each component individually
- Pre/post increment and decrement (`++x`, `x--`, etc.) — these read then write, so the write
  they produce can itself become dead if immediately overwritten

The analysis recurses into branch bodies (`if`/`else`, loops, `try` clauses) and modifier bodies
with fresh state so intra-body pairs are still caught. Conditional boundaries (`&&`, `||`,
ternary) are handled conservatively to avoid false positives across short-circuit paths. Compound
assignments (`+=`, `|=`, etc.) and index/member writes (`mapping[k]`, `struct.field`) are
excluded to avoid false positives.

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
