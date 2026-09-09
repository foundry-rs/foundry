# Right-to-left override character

**Severity**: `High`
**ID**: `rtlo`

## What it does

Detects the right-to-left override codepoint (`U+202E`) and other bidirectional control characters
embedded in identifiers, strings, and comments.

## Why is this bad?

These characters render source code in a different visual order than how the compiler reads it,
allowing an attacker to make malicious code look benign on review.

## Example

```solidity
// transfer(victim‮, attacker)/*  // U+202E hidden between args
```

Use instead:

```solidity
// Avoid bidirectional override characters in code and comments.
```
