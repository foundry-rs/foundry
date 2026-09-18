# Struct names should use `PascalCase`

**Severity**: `Info`
**ID**: `pascal-case-struct`

## What it does

Reports `struct` identifiers longer than one character that do not match the `PascalCase`
convention. Single-character names are not checked.

The configured `mixed_case_exceptions` also permits uppercase acronym patterns such as `ERC20`
inside otherwise PascalCase names, for example `ERC20Data`.

## Why restrict this?

The Solidity style guide recommends `PascalCase` for type-like names (contracts, structs,
enums, libraries). Consistent casing makes code easier to scan and integrates with editor
features and external tooling.

Existing public types or generated declarations may follow another convention. Preserve those
names when compatibility or consistency matters more than adopting this style.

## Example

```solidity
struct user_info { uint256 balance; }
struct USERINFO   { uint256 balance; }
```

Use instead:

```solidity
struct UserInfo { uint256 balance; }
```

## Configuration

Set `mixed_case_exceptions` under `[lint.lint_specific]` in `foundry.toml` to replace the default
uppercase acronym patterns shared with the
[`mixed-case-function`](https://getfoundry.sh/forge/linting/mixed-case-function) lint:

```toml
[lint.lint_specific]
mixed_case_exceptions = ["ERC", "URI", "NFT"]
```
