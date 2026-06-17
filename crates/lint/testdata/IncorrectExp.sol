// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Tests for the `incorrect-exp` lint: `^` (bitwise xor) used between integer literals where `**`
// (exponentiation) was meant.

contract IncorrectExp {
    function bad() external pure returns (uint256 a, uint256 b, uint256 c) {
        a = 10 ^ 18; //~WARN: `^` is bitwise xor, not exponentiation; use `**`
        b = 2 ^ 64; //~WARN: `^` is bitwise xor, not exponentiation; use `**`
        c = (3) ^ (4); //~WARN: `^` is bitwise xor, not exponentiation; use `**`
    }

    function ok(uint256 x) external pure returns (uint256 a, uint256 b, uint256 c) {
        a = 0xff ^ 0x0f; // hex xor: legitimate bit manipulation
        b = x ^ 1; // a variable operand, not two literals
        c = 2 ** 64; // actual exponentiation
    }
}
