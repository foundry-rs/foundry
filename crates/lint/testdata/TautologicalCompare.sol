// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Tests for the `tautological-compare` lint, which flags a relational or equality comparison whose
// two sides are the same side-effect-free expression (always true or false).

type Weird is uint256;

function weirdEq(Weird a, Weird b) pure returns (bool) {
    return Weird.unwrap(a) + Weird.unwrap(b) > 10;
}

using {weirdEq as ==} for Weird global;

contract TautologicalCompare {
    mapping(uint256 => uint256) internal m;

    function pick(uint256 v) internal view returns (uint256) {
        return m[v];
    }

    function bad(uint256 x, uint256 y, uint256 i, uint256[] calldata arr) external view {
        require(x >= x); //~WARN: comparing an expression with itself is always true or false
        require(y == y); //~WARN: comparing an expression with itself is always true or false
        if (arr[i] < arr[i]) {} //~WARN: comparing an expression with itself is always true or false
        if (m[x] != m[x]) {} //~WARN: comparing an expression with itself is always true or false
        if (msg.sender == msg.sender) {} //~WARN: comparing an expression with itself is always true or false
        require(x <= (x)); //~WARN: comparing an expression with itself is always true or false
        if (arr[0] < arr[0]) {} //~WARN: comparing an expression with itself is always true or false
        if (m[1] != m[1]) {} //~WARN: comparing an expression with itself is always true or false
    }

    function ok(uint256 x, uint256 y, uint256 i, uint256 j, uint256[] calldata arr) external view {
        require(x >= y); // different identifiers
        if (arr[i] < arr[j]) {} // different index
        require(pick(x) == pick(x)); // calls excluded: sides may differ
        require(x + 1 > x); // not a self-comparison
    }

    // A user-defined `==` (see `using {weirdEq as ==}`) dispatches to `weirdEq`, which may return
    // either value, so the comparison is not tautological and must not be flagged.
    function okUserDefinedOperator(Weird w) external pure {
        require(w == w);
    }
}
