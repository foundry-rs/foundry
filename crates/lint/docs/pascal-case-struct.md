# Struct names should use PascalCase

**Severity**: `Info`
**ID**: `pascal-case-struct`

Flags struct definitions whose names do not follow `PascalCase`.

## What it does

Reports `struct` identifiers longer than one character that do not match the `PascalCase`
convention. Single-character names are not checked.

## Why is this bad?

The Solidity style guide recommends `PascalCase` for type-like names (contracts, structs,
enums, libraries). Consistent casing makes code easier to scan and integrates with editor
features and external tooling.

## Example

### Bad

```solidity
struct user_info { uint256 balance; }
struct USERINFO   { uint256 balance; }
```

### Good

```solidity
struct UserInfo { uint256 balance; }
```
