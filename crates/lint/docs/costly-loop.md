# Costly operations inside a loop

**Severity**: `Gas`
**ID**: `costly-loop`

Flags storage variable writes inside loops. Each SSTORE costs at least 2,900 gas (warm) or 20,000
gas (cold), so writing to storage on every loop iteration can be extremely expensive. Accumulating
the result in a local memory variable and writing to storage once after the loop is the standard
optimization.

## What it does

Reports assignments, compound assignments, increments/decrements, and `delete` expressions that
directly write to a storage variable inside any `for`, `while`, or `do-while` loop body, including
writes through storage array indices and mapping keys.

## Why is this bad?

SSTORE is one of the most expensive EVM opcodes. Writing to storage in a loop multiplies that cost
by the number of iterations and can easily cause transactions to run out of gas or become
economically impractical.

## Example

### Bad

```solidity
contract C {
    uint256 public counter;

    function bad(uint256 n) external {
        for (uint256 i = 0; i < n; i++) {
            counter++; // costly-loop: SSTORE on every iteration
        }
    }
}
```

### Good

```solidity
contract C {
    uint256 public counter;

    function good(uint256 n) external {
        uint256 local = counter;
        for (uint256 i = 0; i < n; i++) {
            local++;
        }
        counter = local; // single SSTORE after the loop
    }
}
```

## Notes

This is a `Gas`-severity lint and is **not** applied to test or script files.
