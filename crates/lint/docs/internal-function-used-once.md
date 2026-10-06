# Internal function used once

**Severity**: `Info`
**ID**: `internal-function-used-once`

## What it does

Reports internal and free functions referenced exactly once across the compiled sources.

Functions starting with `_`, virtual functions, overrides, recursive functions, and
user-defined operator functions are excluded.

## Why restrict this?

A function with a single caller introduces another declaration for the reader to follow. Inlining
short helpers can make the caller easier to read. A helper can still be useful with one caller
when its name explains a distinct operation or separates complex logic; review that tradeoff
before inlining it.

## Example

```solidity
function price(uint256 amount) internal view returns (uint256) {
    return scaled(amount) * rate;
}

function scaled(uint256 amount) internal pure returns (uint256) {
    return amount * 1e18;
}
```

Use instead:

```solidity
function price(uint256 amount) internal view returns (uint256) {
    return amount * 1e18 * rate;
}
```
