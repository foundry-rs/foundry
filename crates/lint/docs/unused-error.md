# Unused error

**Severity**: `Info`
**ID**: `unused-error`

Flags a custom error declaration that is never referenced anywhere in the compiled sources.

## What it does

Reports each `error` declaration whose symbol is never referenced. Any resolved reference counts
as a use: a `revert Err(...)` statement (including the qualified `revert Lib.Err(...)`),
`require(cond, Err(...))` (Solidity 0.8.26+), and `Err.selector` (including through
`abi.encodeWithSelector`). Uses are collected across the whole compilation unit,
so an error declared in a shared file and reverted elsewhere in the project is not reported.

Errors declared in interfaces and abstract contracts are not reported: they are ABI surface
meant for implementers and off-chain consumers, which may live outside the compiled sources.

## Why is this bad?

An unused error is dead code. It suggests a missing revert path or a leftover from a refactor.

## Example

### Bad

```solidity
error Unauthorized(); // declared but never referenced
```

### Good

```solidity
error Unauthorized();

function withdraw() external {
    if (msg.sender != owner) revert Unauthorized();
}
```

## Limitations

A selector hardcoded in inline assembly or built with `abi.encodeWithSignature("Err(...)")` does
not reference the declaration and is not seen as a use.
