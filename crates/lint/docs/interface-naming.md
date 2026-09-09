# Interface name should be prefixed with 'I'

**Severity**: `Info`
**ID**: `interface-naming`

Flags `interface` declarations whose names are not prefixed with `I`.

## What it does

Reports `interface Foo` where `Foo` does not start with `I` (e.g. `IFoo`).

## Why is this bad?

Prefixing interfaces with `I` is the prevailing convention in Solidity codebases (`IERC20`,
`IERC721`, `IUniswapV2Pair`, ...). Following it makes the role of each type unambiguous at use
sites and aligns with the matching
[`interface-file-naming`](https://getfoundry.sh/forge/linting/interface-file-naming) lint.

## Example

### Bad

```solidity
interface ERC20 { /* ... */ }
```

### Good

```solidity
interface IERC20 { /* ... */ }
```
