# Cyclomatic complexity

**Severity**: `Info`
**ID**: `cyclomatic-complexity`

## What it does

Reports functions with a complexity score above 11. The score starts at one and increases
for each decision point: `if`, a loop with a condition, a ternary, a `catch` clause, or
an additional assembly `switch` case. Boolean `&&` and `||` operators do not add to the score.

## Why restrict this?

A function with many decision points can be harder to read, review, and test. Splitting it into
smaller functions can make each piece easier to understand. Complexity is a heuristic, not a count
of all possible execution paths; a cohesive dispatch function may be clearer kept together.

## Example

```solidity
// complexity 12: eleven branching points plus one
function dispatch(uint256 kind) internal pure returns (uint256) {
    if (kind == 0) return 10;
    if (kind == 1) return 20;
    if (kind == 2) return 30;
    if (kind == 3) return 40;
    if (kind == 4) return 50;
    if (kind == 5) return 60;
    if (kind == 6) return 70;
    if (kind == 7) return 80;
    if (kind == 8) return 90;
    if (kind == 9) return 100;
    if (kind == 10) return 110;
    revert();
}
```

Use instead:

```solidity
function dispatch(uint256 kind) internal pure returns (uint256) {
    if (kind < 6) return dispatchLow(kind);
    return dispatchHigh(kind);
}

function dispatchLow(uint256 kind) internal pure returns (uint256) {
    if (kind == 0) return 10;
    if (kind == 1) return 20;
    if (kind == 2) return 30;
    if (kind == 3) return 40;
    if (kind == 4) return 50;
    if (kind == 5) return 60;
    revert();
}

function dispatchHigh(uint256 kind) internal pure returns (uint256) {
    if (kind == 6) return 70;
    if (kind == 7) return 80;
    if (kind == 8) return 90;
    if (kind == 9) return 100;
    if (kind == 10) return 110;
    revert();
}
```
