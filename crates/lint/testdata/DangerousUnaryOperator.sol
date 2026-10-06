//@compile-flags: --only-lint dangerous-unary-operator
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Tests for `dangerous-unary-operator`: an assignment whose `=` is fused to a unary operator
// (`=-`, `=~`) parses as a plain assignment of a unary expression (`x = -1`), not the compound
// assignment (`x -= 1`) it resembles. The fused unary may also lead a larger RHS (`x =- a + 1`
// parses as `x = (-a) + 1`). Spaced unary assignments and real compound operators are left alone.
// `=+` is not testable: unary `+` is a parse error in modern Solidity.

contract DangerousUnaryOperator {
    function bad(int256 x, uint256 y)
        external
        pure
        returns (int256 a, uint256 b, int256 c, int256 d, int256 e, int256 f)
    {
        a =- 1; //~WARN: unary operator fused to
        b =~ y; //~WARN: unary operator fused to
        c =-x; //~WARN: unary operator fused to
        d =- x + 1; //~WARN: unary operator fused to
        e =- x * 2; //~WARN: unary operator fused to
        f /* note */=- 1; //~WARN: unary operator fused to
    }

    function ok(int256 x, uint256 y)
        external
        pure
        returns (int256 a, uint256 b, int256 c, int256 d, int256 e, int256 f)
    {
        a = -1; // spaced negation: intentional
        b = ~y; // spaced bitwise not: intentional
        c -= 1; // real compound assignment
        d = x - 1; // subtraction: the RHS does not start with a unary operator
        e = -x + 1; // spaced negation leading a binary RHS: intentional, must not flag
        f =/* note */- 1; // a comment between `=` and the unary breaks the fusion
    }
}
