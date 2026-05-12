# Inconsistent pragma directives

**Severity**: `Info`
**ID**: `pragma-inconsistent`

Flags projects whose source files declare incompatible or differently-shaped Solidity version
pragmas.

## What it does

Inspects every `pragma solidity ...;` directive across all input source files and reports when
their version requirements are inconsistent (different exact versions, mixed caret/tilde/range
shapes, etc.).

## Why is this bad?

A project compiled under multiple Solidity versions can subtly change behavior between files
(e.g. checked arithmetic, default visibility, ABI encoding). Aligning pragmas across the project
removes a hidden source of integration bugs and makes upgrades coordinated.

## Example

### Bad

```solidity
// A.sol
pragma solidity 0.8.18;

// B.sol
pragma solidity ^0.8.20;

// C.sol
pragma solidity >=0.7.0 <0.9.0;
```

### Good

```solidity
// All files
pragma solidity 0.8.20;
```
