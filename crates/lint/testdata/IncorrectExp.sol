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
        returns (uint256 a, uint256 b, uint256 c, uint256 d, uint256 e, uint256 f, uint256 g)
    {
        a = 255 ^ 128; // decimal bitmask (0xFF ^ 0x80), base is not 2 or 10
        b = 170 ^ 85; // decimal bitmask (0xAA ^ 0x55), base is not 2 or 10
        c = 3 ^ 4; // base 3: not a base written as a power
        d = 0xff ^ 0x0f; // hex xor: legitimate bit manipulation
        e = x ^ 1; // a variable operand, not two literals
        f = 2 ** 64; // actual exponentiation
        g = id(10) ^ 18; // a non-cast call, not a literal: must not be unwrapped
    }
}
