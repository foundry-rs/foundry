# Struct names should use PascalCase

**Severity**: `Info`
**ID**: `pascal-case-struct`

Flags struct definitions whose names do not follow `PascalCase`.

## What it does

Reports any `struct` whose identifier does not match the `PascalCase` convention.

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
