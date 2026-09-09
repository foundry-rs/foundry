# Inconsistent type names

**Severity**: `Low`
**ID**: `inconsistent-type-names`

## What it does

Reports shorthand `uint` or `int` declarations when the same contract also uses `uint256`
or `int256`, respectively. This includes types within arrays and mappings.

Each contract is checked separately. A contract that consistently uses shorthand types
is not reported.

## Why restrict this?

`uint` and `uint256` compile to the same type, as do `int` and `int256`, so mixing their spellings
does not change runtime behavior. It does make the code less consistent and can make readers wonder
whether an omitted size was intentional. Using the explicit spelling throughout removes that
ambiguity.

A project may consistently prefer shorthand integer types. When integrating an established API
or generated declarations, preserve that convention and suppress mixed spelling after review.

## Example

```solidity
contract Vault {
    uint public shares;
    uint256 public assets;
}
```

Use instead:

```solidity
contract Vault {
    uint256 public shares;
    uint256 public assets;
}
```
