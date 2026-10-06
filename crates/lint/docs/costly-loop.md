# Costly operations inside a loop

**Severity**: `Gas`
**ID**: `costly-loop`

## What it does

Reports assignments, compound assignments, increments/decrements, and `delete` expressions that
directly write to a storage variable inside any `for`, `while`, or `do-while` loop body, including
writes through storage array indices and mapping keys.

## Why is this bad?

Repeated storage writes can be expensive; their cost depends on slot access history and the
original, current, and new values. Accumulating the result in a local variable and writing to
storage once after the loop can reduce that work.

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
