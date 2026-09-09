# Multiple contracts in one file

**Severity**: `Info`
**ID**: `multi-contract-file`

## What it does

Reports every non-exempt top-level `contract`, `interface`, or `library` definition in a
file that contains more than one non-exempt declaration.

## Why restrict this?

Keeping one contract per file can improve discoverability and make import paths predictable.
Closely related interfaces, helper contracts, or test fixtures can also be reasonable to group.
File organization alone does not add unrelated contracts to a deployed contract's bytecode.

## Example

```solidity
// File: Token.sol
contract TokenA { /* ... */ }
contract TokenB { /* ... */ }
```

Use instead:

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
