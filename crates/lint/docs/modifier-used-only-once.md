# Modifier used only once

**Severity**: `Info`
**ID**: `modifier-used-only-once`

Flags modifiers invoked by exactly one function in the whole compilation unit.

## What it does

Reports a modifier that exactly one function invokes, constructors included. Invocations are taken from the resolved modifier lists, so base-constructor calls sitting in the same syntactic position are never confused with modifier calls, and each invocation is attributed to the declaration the compiler selected. Invocations are counted across dependencies too, while only modifiers declared in the project's own sources report. Aderyn's detector of the same name counts invocations the same way and does not exempt virtual modifiers or overrides.

Out of scope: `virtual` modifiers and overrides (they exist for dynamic dispatch, so inlining them is not an option), and modifiers never invoked, which are dead code rather than an inlining candidate.

## Why restrict this?

A modifier with a single user introduces a separate declaration for the reader to follow. Moving
simple checks into the function can make its behavior easier to see. Keeping a named modifier may
still clarify an access-control policy or enforce a project convention; a single use is not
itself a correctness issue.

## Example

```solidity
modifier onlyOwner() {
    require(msg.sender == owner, "not owner");
    _;
}

function withdraw() external onlyOwner {
    // ...
}
```

Use instead:

```solidity
function withdraw() external {
    require(msg.sender == owner, "not owner");
    // ...
}
```
