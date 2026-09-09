# Interface name should be prefixed with `I`

**Severity**: `Info`
**ID**: `interface-naming`

Flags `interface` declarations whose names are not prefixed with `I`.

## What it does

Reports `interface Foo` where `Foo` does not start with `I` (e.g. `IFoo`).

## Why restrict this?

Prefixing interfaces with `I` is the prevailing convention in Solidity codebases (`IERC20`,
`IERC721`, `IUniswapV2Pair`, ...). Following it makes the role of each type unambiguous at use
sites and aligns with the matching
[`interface-file-naming`](https://getfoundry.sh/forge/linting/interface-file-naming) lint.

A project may use a different interface-naming convention. Preserve established public type names
when renaming would disrupt downstream imports or conflict with that convention.

## Example

```solidity
interface ERC20 { /* ... */ }
```

Use instead:

```solidity
interface IERC20 { /* ... */ }
```
