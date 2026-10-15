# Inconsistent pragma directives

**Severity**: `Info`
**ID**: `pragma-inconsistent`

## What it does

Reports inconsistent `pragma solidity ...;` requirements across source files, such as different
exact versions or mixed caret, tilde, and range constraints.

## Why restrict this?

Different constraints can complicate compiler upgrades and make separately compiled parts of a
project use different language behavior. Aligning them can make upgrades easier to coordinate.
Different but overlapping constraints can still select the same compiler; reusable libraries
may intentionally support a wider range than an application. This lint does not prove that the
requirements are incompatible or that different compilers were used.

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
