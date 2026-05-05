# Interface file naming

**Severity**: `Info`
**ID**: `interface-file-naming`

Flags Solidity files whose only top-level declaration is an interface but whose filename is not
prefixed with `I`.

## What it does

Reports interface-only files whose path basename does not start with `I` (e.g. `IERC20.sol`).

## Why is this bad?

Prefixing interface filenames with `I` is the prevailing convention in the Solidity ecosystem.
Following it makes import paths predictable and lets reviewers tell at a glance whether they are
looking at an interface or an implementation.

## Example

### Bad

```text
contracts/Token.sol      // file contains only `interface Token { ... }`
```

### Good

```text
contracts/IToken.sol     // file contains only `interface IToken { ... }`
```
