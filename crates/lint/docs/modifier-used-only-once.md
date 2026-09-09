# Modifier used only once

**Severity**: `Info`
**ID**: `modifier-used-only-once`

## What it does

Reports modifiers used by exactly one function or constructor across the compiled sources.
Virtual modifiers, overrides, and unused modifiers are excluded.

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
