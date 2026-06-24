# Numeric literal with too many digits

**Severity**: `Info`
**ID**: `too-many-digits`

Flags numeric literals containing five or more consecutive zeros, which are easy to misread.

## What it does

Reports decimal numeric literals that contain a run of 5 or more `0` characters.

## Why is this bad?

Long sequences of zeros are difficult to count visually, and an off-by-one zero is a common bug
(e.g. funding `1_000_000` instead of `10_000_000`). Use scientific notation, sub-denominations, or
underscore separators to make the magnitude obvious.

## Example

### Bad

```solidity
uint256 amount = 1000000000000000000;
```

### Good

```solidity
uint256 amount = 1e18;
uint256 amount2 = 1 ether;
uint256 amount3 = 1_000_000_000_000_000_000;
```
