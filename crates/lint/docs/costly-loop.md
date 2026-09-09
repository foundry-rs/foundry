# Costly operations inside a loop

**Severity**: `Gas`
**ID**: `costly-loop`

Flags storage variable writes inside loops. Repeated storage writes can be expensive; their cost
depends on slot access history and the original, current, and new values. Accumulating the result
in a local variable and writing to storage once after the loop can reduce that work.

## What it does

Reports assignments, compound assignments, increments/decrements, and `delete` expressions that
directly write to a storage variable inside any `for`, `while`, or `do-while` loop body, including
writes through storage array indices and mapping keys.

## Why is this bad?

SSTORE is one of the most expensive EVM opcodes. Writing to storage in a loop multiplies that cost
by the number of iterations and can easily cause transactions to run out of gas or become
economically impractical.

## Example

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

Use instead:

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
