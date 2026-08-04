# Multiple contracts in one file

**Severity**: `Info`
**ID**: `multi-contract-file`

Flags source files that declare more than one top-level contract, interface, or library.

## What it does

Reports each top-level `contract`, `interface`, or `library` definition (after the first) in a
file that contains more than one such declaration.

## Why is this bad?

Keeping one contract per file improves discoverability (`grep`, IDE jump-to-file), simplifies
import paths, and avoids unintentional bytecode bloat from artifacts that bundle unrelated
contracts.

## Example

### Bad

```solidity
// File: Token.sol
contract TokenA { /* ... */ }
contract TokenB { /* ... */ }
```

### Good

```solidity
// File: TokenA.sol
contract TokenA { /* ... */ }

// File: TokenB.sol
contract TokenB { /* ... */ }
```

## Configuration

Set `multi_contract_file_exceptions` under `[lint.lint_specific]` in `foundry.toml` to allow
multiple interfaces, libraries, or abstract contracts in one file. Regular contracts cannot be
exempted.

```toml
[lint.lint_specific]
multi_contract_file_exceptions = ["interface", "library", "abstract_contract"]
```
