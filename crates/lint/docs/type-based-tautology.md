# Type-Based Tautology

**Severity**: `Med`
**ID**: `type-based-tautology`

## What it does

Flags comparisons that are always true or false because of an integer type's range,
such as `uint256 x >= 0`. Also reports conditions that cover the entire range, such as
`x > 0 || x == 0` for unsigned `x`.

Single comparisons use the expression's checked integer type, including struct fields,
array elements, and function return values. Combined comparisons must refer to the same
local or state variable, optionally cast to an integer type.

## Why is this bad?

A condition that is permanently true contributes no useful logic and may hide a bug where the developer intended to compare against a different value or use a differently sized type. A condition that is permanently false creates unreachable code, which can silently suppress intended behavior such as access control checks or error handling.

## Example

```solidity
function isValid(uint256 x) public pure returns (bool) {
    return x >= 0; // always true, uint cannot be negative
}

function isInRange(uint8 x) public pure returns (bool) {
    return x < 256; // always true, uint8 max is 255
}

function isBelowMin(int8 x) public pure returns (bool) {
    return x < -128; // always false, int8 min is -128
}

function isImpossible(uint8 x) public pure returns (bool) {
    return x == 256; // always false, 256 is outside uint8 range
}

function coversRange(uint256 x) public pure returns (bool) {
    return x > 0 || x == 0; // always true, every uint256 is either zero or greater
}
```

Use instead:

```solidity
function isValid(uint256 x) public pure returns (bool) {
    return x > 0; // meaningful: false when x == 0
}

function isInRange(uint8 x, uint8 limit) public pure returns (bool) {
    return x < limit; // compare against a runtime value
}

function isBelowThreshold(int8 x) public pure returns (bool) {
    return x < -100; // a value within the representable range
}
```
