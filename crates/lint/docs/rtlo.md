# Right-to-left override character

**Severity**: `High`
**ID**: `rtlo`

Flags the presence of Unicode bidirectional override characters in source code, which can be used
to hide malicious behavior ("Trojan Source", [CVE-2021-42574](https://cve.mitre.org/cgi-bin/cvename.cgi?name=CVE-2021-42574)).

## What it does

Detects the right-to-left override codepoint (`U+202E`) and other bidirectional control characters
embedded in identifiers, strings, and comments.

## Why is this bad?

These characters render source code in a different visual order than how the compiler reads it,
allowing an attacker to make malicious code look benign on review. Solidity contracts are public
and frequently audited visually; this attack vector must not be ignored.

## Example

### Bad

```solidity
// transfer(victim‮, attacker)/*  // U+202E hidden between args
```

### Good

```solidity
// Avoid bidirectional override characters in code and comments.
```
