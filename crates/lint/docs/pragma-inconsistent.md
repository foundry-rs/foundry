# Inconsistent pragma directives

**Severity**: `Info`
**ID**: `pragma-inconsistent`

## What it does

Reports inconsistent `pragma solidity ...;` requirements across source files, such as different
exact versions or mixed caret, tilde, and range constraints.

## Why is this bad?

A project compiled under multiple Solidity versions can subtly change behavior between files
(e.g. checked arithmetic, default visibility, ABI encoding). Aligning pragmas across the project
removes a hidden source of integration bugs and makes upgrades coordinated.

## Example

```solidity
// A.sol
pragma solidity 0.8.18;

// B.sol
pragma solidity ^0.8.20;

// C.sol
pragma solidity >=0.7.0 <0.9.0;
```

Use instead:

```solidity
// All files
pragma solidity 0.8.20;
```
