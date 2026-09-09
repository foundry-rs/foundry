# Unused error

**Severity**: `Info`
**ID**: `unused-error`

Flags a custom error declaration that is never referenced anywhere in the compiled sources.

## What it does

Reports custom error declarations that are never used by a revert, `require`, or selector
reference anywhere in the compiled sources.

Errors in interfaces and abstract contracts are excluded because external implementations
and consumers may rely on their ABI.

## Why is this bad?

An unused error is dead code. It suggests a missing revert path or a leftover from a refactor.

## Example

```solidity
error Unauthorized(); // declared but never referenced
```

Use instead:

```solidity
error Unauthorized();

function withdraw() external {
    if (msg.sender != owner) revert Unauthorized();
}
```

## Limitations

A selector hardcoded in inline assembly or built with `abi.encodeWithSignature("Err(...)")` does
not reference the declaration and is not seen as a use.
