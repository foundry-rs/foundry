# Type-Based Tautology

**Severity**: `Med`
**ID**: `type-based-tautology`

Detects comparison expressions that are always true or always false due to the numeric range of the variable's type. These dead conditions indicate logic errors or misunderstandings about integer bounds.

## What it does

Flags binary comparisons (`<`, `<=`, `>`, `>=`) where one operand is a typed integer variable and the other is a constant that lies outside, or exactly at the boundary of the variable's representable range, making the condition unconditionally true or false.

Examples:
- `uint x >= 0` is always true because unsigned integers cannot be negative.
- `uint8 x > 255` is always false because 255 is the maximum value of `uint8`.
- `int8 x < -128` is always false because -128 is the minimum value of `int8`.

The check also applies to explicit type casts: `uint8(x) < 256` is always true.

## Why is this bad?

A condition that is permanently true contributes no useful logic and may hide a bug where the developer intended to compare against a different value or use a differently sized type. A condition that is permanently false creates unreachable code, which can silently suppress intended behavior such as access control checks or error handling.

## Example

### Bad

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
```

### Good

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
