# Cyclomatic complexity

**Severity**: `Info`
**ID**: `cyclomatic-complexity`

Flags functions whose cyclomatic complexity is strictly above 11.

## What it does

Reports a function whose cyclomatic complexity exceeds 11, the threshold Slither's detector of the same name uses. The complexity is one plus the number of decision points in the body: each `if` (loop conditions included, since every `for`, `while` and `do while` branches on its condition, and a condition-less `for (;;)` adds nothing), each ternary, each `catch` clause and each additional case of an assembly `switch`. Boolean `&&` and `||` operators add nothing, matching the control-flow graph Slither computes on.

## Why is this bad?

A function with many independent paths is hard to read, hard to review and hard to test: covering it takes one test per path, and every new branch multiplies the states a reader has to keep in mind. Splitting it into smaller functions gives each piece a testable contract.

## Example

### Bad

```solidity
// complexity 12: eleven branching points plus one
function dispatch(uint256 kind) internal {
    if (kind == 0) { ... }
    if (kind == 1) { ... }
    // ... nine more branches
}
```

### Good

```solidity
function dispatch(uint256 kind) internal {
    if (kind < 8) {
        dispatchLow(kind);
    } else {
        dispatchHigh(kind);
    }
}
```
