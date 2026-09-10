# Interface file naming

**Severity**: `Info`
**ID**: `interface-file-naming`

## What it does

Reports interface-only files whose path basename does not start with `I` (e.g. `IERC20.sol`).

## Why restrict this?

Prefixing interface filenames with `I` is the prevailing convention in the Solidity ecosystem.
Following it makes import paths predictable and lets reviewers tell at a glance whether they are
looking at an interface or an implementation.

A project may use a different file-naming convention, or preserve existing import paths for
compatibility. In those cases, keep the filename and suppress the lint.

## Example

```solidity
// File: contracts/Token.sol
interface IToken {}
```

Use instead:

```solidity
// File: contracts/IToken.sol
interface IToken {}
```
