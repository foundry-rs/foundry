# Contract name reused across files

**Severity**: `Info`
**ID**: `name-reused`

Flags contract, interface, or library names that are defined in more than one source file in the
project.

## What it does

Scans all input source files and reports every contract-like item whose name collides with one
defined in a different file, listing the conflicting paths in the diagnostic message.

## Why is this bad?

When multiple files define a type with the same name, the compiler, toolchain, and developers must
rely on import paths to disambiguate them. This creates subtle risks:

- Deployment scripts or tests that reference a contract by name alone may silently target the wrong
  artifact.
- Artifact directories (e.g. `out/`) contain one JSON file per contract name, so a duplicate name
  causes one build output to overwrite the other.
- Code reviewers and auditors can be misled about which implementation is actually in scope.

## Example

### Bad

```solidity
// src/Token.sol
pragma solidity ^0.8.0;

contract Token {
    // production ERC-20
}

// test/Token.sol
pragma solidity ^0.8.0;

contract Token {
    // mock with unrestricted minting — same name, different behavior
}
```

### Good

```solidity
// src/Token.sol
pragma solidity ^0.8.0;

contract Token {
    // production ERC-20
}

// test/MockToken.sol
pragma solidity ^0.8.0;

contract MockToken {
    // mock with unrestricted minting
}
```
