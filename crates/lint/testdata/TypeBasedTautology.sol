//@compile-flags: --only-lint type-based-tautology

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract TypeBasedTautology {
    // --- uint: comparisons against 0 ---

    function uintGeZero(uint256 x) public pure returns (bool) {
        return x >= 0; //~WARN: condition is always true or false based on the variable's type
    }

    function uintLtZero(uint256 x) public pure returns (bool) {
        return x < 0; //~WARN: condition is always true or false based on the variable's type
    }

    function uintGtZero(uint256 x) public pure returns (bool) {
        return x > 0; // ok – can be false when x == 0
    }

    function uintLeZero(uint256 x) public pure returns (bool) {
        return x <= 0; // ok, equivalent to x == 0, not a tautology
    }

    // --- uint: comparisons with out-of-range constants ---

    function uint8LtMax256(uint8 x) public pure returns (bool) {
        return x < 256; //~WARN: condition is always true or false based on the variable's type
    }

    function uint8GtMax255(uint8 x) public pure returns (bool) {
        return x > 255; //~WARN: condition is always true or false based on the variable's type
    }

    function uint8GeMax256(uint8 x) public pure returns (bool) {
        return x >= 256; //~WARN: condition is always true or false based on the variable's type
    }

    function uint8LeMax255(uint8 x) public pure returns (bool) {
        return x <= 255; //~WARN: condition is always true or false based on the variable's type
    }

    function uint8GtMax254(uint8 x) public pure returns (bool) {
        return x > 254; // ok – false when x == 254 or below
    }

    function uint8LtMax255(uint8 x) public pure returns (bool) {
        return x < 255; // ok – false when x == 255
    }

    // --- flipped operands (constant on left) ---

    function zeroGtUint(uint256 x) public pure returns (bool) {
        return 0 > x; //~WARN: condition is always true or false based on the variable's type
    }

    function zeroLeUint(uint256 x) public pure returns (bool) {
        return 0 <= x; //~WARN: condition is always true or false based on the variable's type
    }

    function val256GtUint8(uint8 x) public pure returns (bool) {
        return 256 > x; //~WARN: condition is always true or false based on the variable's type
    }

    function val256LeUint8(uint8 x) public pure returns (bool) {
        return 256 <= x; //~WARN: condition is always true or false based on the variable's type
    }

    // --- int: boundary comparisons ---

    function int8GeMin(int8 x) public pure returns (bool) {
        return x >= -128; //~WARN: condition is always true or false based on the variable's type
    }

    function int8LtMin(int8 x) public pure returns (bool) {
        return x < -128; //~WARN: condition is always true or false based on the variable's type
    }

    function int8GtMax(int8 x) public pure returns (bool) {
        return x > 127; //~WARN: condition is always true or false based on the variable's type
    }

    function int8LeMax(int8 x) public pure returns (bool) {
        return x <= 127; //~WARN: condition is always true or false based on the variable's type
    }

    function int8GeAlmostMin(int8 x) public pure returns (bool) {
        return x >= -127; // ok
    }

    function int8LeAlmostMax(int8 x) public pure returns (bool) {
        return x <= 126; // ok
    }

    // --- explicit casts ---

    function castedUint8GtMax(uint256 raw) public pure returns (bool) {
        // forge-lint: disable-next-line(unsafe-typecast)
        return uint8(raw) > 255; //~WARN: condition is always true or false based on the variable's type
    }

    function castedInt8LtMin(int256 raw) public pure returns (bool) {
        // forge-lint: disable-next-line(unsafe-typecast)
        return int8(raw) < -128; //~WARN: condition is always true or false based on the variable's type
    }

    // --- eq / ne with out-of-range constants ---
    // (solc rejects comparisons with sign-mismatched literals, e.g. uint == -1 or int8 == 128)

    function uint8EqOutOfRange(uint8 x) public pure returns (bool) {
        return x == 256; //~WARN: condition is always true or false based on the variable's type
    }

    function uint8NeOutOfRange(uint8 x) public pure returns (bool) {
        return x != 256; //~WARN: condition is always true or false based on the variable's type
    }

    function int8EqBelowMin(int8 x) public pure returns (bool) {
        return x == -129; //~WARN: condition is always true or false based on the variable's type
    }

    function uint8EqInRange(uint8 x) public pure returns (bool) {
        return x == 255; // ok, 255 is the maximum of uint8, not out-of-range
    }

    function int8EqAtMin(int8 x) public pure returns (bool) {
        return x == -128; // ok, -128 is within int8 range
    }

    // --- boolean compositions over type boundaries ---

    function uintLowerBoundaryOr(uint256 x) public pure returns (bool) {
        return x > 0 || x == 0; //~WARN: condition is always true or false based on the variable's type
    }

    function uintUpperBoundaryOr(uint8 x) public pure returns (bool) {
        return x < 255 || x == 255; //~WARN: condition is always true or false based on the variable's type
    }

    function intLowerBoundaryOr(int8 x) public pure returns (bool) {
        return x > -128 || x == -128; //~WARN: condition is always true or false based on the variable's type
    }

    function intUpperBoundaryOr(int8 x) public pure returns (bool) {
        return x < 127 || x == 127; //~WARN: condition is always true or false based on the variable's type
    }

    function interiorBoundaryOr(int8 x) public pure returns (bool) {
        return x > 0 || x == 0; // ok, negative values are not covered
    }

    function differentVariableOr(uint256 x, uint256 y) public pure returns (bool) {
        return x > 0 || y == 0; // ok, the comparisons refer to different values
    }

    // --- boundary compositions over explicit casts ---

    function castedUint8LowerBoundaryOr(uint256 raw) public pure returns (bool) {
        // forge-lint: disable-next-line(unsafe-typecast)
        return uint8(raw) > 0 || uint8(raw) == 0; //~WARN: condition is always true or false based on the variable's type
    }

    function uint8OppositeBoundaryOr(uint8 x) public pure returns (bool) {
        return x > 0 || x < 255; //~WARN: condition is always true or false based on the variable's type
    }

    function castedWideningUpperBoundaryOr(uint8 x) public pure returns (bool) {
        return uint16(x) < 255 || uint16(x) == 255; //~WARN: condition is always true or false based on the variable's type
    }

    function castedAndUncastedDifferentRangeOr(uint256 raw) public pure returns (bool) {
        // forge-lint: disable-next-line(unsafe-typecast)
        return uint8(raw) > 0 || raw == 0; // ok -- the effective ranges differ
    }

    function nestedCastedDifferentRangeOr(uint256 raw) public pure returns (bool) {
        // forge-lint: disable-next-line(unsafe-typecast)
        return uint256(uint8(raw)) > 0 || raw == 0; // ok -- the inner cast can change the value
    }

    function uintNegativeZeroLowerBoundaryOr(uint256 x) public pure returns (bool) {
        return x > -0 || x == -0; //~WARN: condition is always true or false based on the variable's type
    }
}
