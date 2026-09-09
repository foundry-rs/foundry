# Numeric literal with too many digits

**Severity**: `Info`
**ID**: `too-many-digits`

## What it does

Reports Solidity and Yul numeric literals that contain a run of 5 or more `0` characters. Decimal
literals with scientific notation, literals with a sub-denomination, and 40-digit hexadecimal
address literals are skipped. Other hexadecimal literals remain in scope because long padded masks
and bit patterns are also difficult to review.

## Why restrict this?

Long sequences of zeros are difficult to count visually, and an off-by-one zero is a common bug
(e.g. funding `1_000_000` instead of `10_000_000`). Use scientific notation, sub-denominations, or
underscore separators to make the magnitude obvious.

Fixed-width literals may intentionally mirror an external format, generated data, or a published
constant. Preserve that representation when changing it would make comparison harder.

## Example

```solidity
uint256 amount = 1000000000000000000;
```

Use instead:

```solidity
uint256 amount = 1e18;
uint256 amount2 = 1 ether;
uint256 amount3 = 1_000_000_000_000_000_000;
```
