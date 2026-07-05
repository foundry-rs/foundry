//@compile-flags: --only-lint incorrect-exp
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Tests for the `incorrect-exp` lint: `^` (bitwise xor) used where `**` (exponentiation) was meant.
// The base is restricted to `2` and `10` (the bases people write as powers), mirroring GCC and
// Clang's `-Wxor-used-as-pow`, so decimal bitmask xors like `255 ^ 128` are not flagged.

contract IncorrectExp {
    function id(uint256 v) internal pure returns (uint256) {
        return v;
    }

    function bad()
        external
        pure
        returns (uint256 a, uint256 b, uint256 c, uint256 d, uint256 e, uint256 f)
    {
        a = 10 ^ 18; //~WARN: `^` is bitwise xor, not exponentiation; use `**`
        b = 2 ^ 64; //~WARN: `^` is bitwise xor, not exponentiation; use `**`
        c = 2 ^ 256; //~WARN: `^` is bitwise xor, not exponentiation; use `**`
        d = uint256(10) ^ 18; //~WARN: `^` is bitwise xor, not exponentiation; use `**`
        e = 2 ^ uint256(64); //~WARN: `^` is bitwise xor, not exponentiation; use `**`
        f = uint256(uint8(2)) ^ 64; //~WARN: `^` is bitwise xor, not exponentiation; use `**`
    }

    function ok(uint256 x)
        external
        pure
        returns (
            uint256 a,
            uint256 b,
            uint256 c,
            uint256 d,
            uint256 e,
            uint256 f,
            uint256 g,
            uint256 h,
            uint256 i,
            bytes32 j,
            uint256 k,
            uint256 l
        )
    {
        a = 255 ^ 128; // decimal bitmask (0xFF ^ 0x80), base is not 2 or 10
        b = 170 ^ 85; // decimal bitmask (0xAA ^ 0x55), base is not 2 or 10
        c = 3 ^ 4; // base 3: not a base written as a power
        d = 0xff ^ 0x0f; // hex xor: legitimate bit manipulation
        e = x ^ 1; // a variable operand, not two literals
        f = 2 ** 64; // actual exponentiation
        g = id(10) ^ 18; // a non-cast call, not a literal: must not be unwrapped
        h = 1e1 ^ 18; // scientific notation base (1e1 evaluates to 10): not a plain integer literal
        i = 10 ^ 1e1; // scientific notation exponent: not a plain integer literal
        j = bytes32(uint256(2)) ^ bytes32(uint256(64)); // bytesN xor: not an integer cast
        k = 2 wei ^ 64; // denominated literal (2 wei == 2): not a plain integer literal
        l = 10 seconds ^ 18; // denominated literal (10 seconds == 10): not a plain integer literal
    }
}
