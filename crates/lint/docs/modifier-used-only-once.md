# Modifier used only once

**Severity**: `Info`
**ID**: `modifier-used-only-once`

Flags modifiers invoked by exactly one function in the whole compilation unit.

## What it does

Reports a modifier that exactly one function invokes, constructors included. Invocations are taken from the resolved modifier lists, so base-constructor calls sitting in the same syntactic position are never confused with modifier calls, and each invocation is attributed to the declaration the compiler selected. Invocations are counted across dependencies too, while only modifiers declared in the project's own sources report. Aderyn's detector of the same name counts invocations the same way and does not exempt virtual modifiers or overrides.

Out of scope: `virtual` modifiers and overrides (they exist for dynamic dispatch, so inlining them is not an option), and modifiers never invoked, which are dead code rather than an inlining candidate.

## Why is this bad?

A modifier with a single user adds indirection without factoring anything: the reader jumps to the declaration to understand one function. Writing the checks at the top of that function reads straighter; if a second function needs them later, extracting the modifier back is mechanical.

## Example

### Bad

```solidity
modifier onlyOwner() {
    require(msg.sender == owner, "not owner");
    _;
}

function withdraw() external onlyOwner {
    ...
}
```

### Good

```solidity
function withdraw() external {
    require(msg.sender == owner, "not owner");
    ...
}
```
