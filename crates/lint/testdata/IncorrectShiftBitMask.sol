//@compile-flags: --only-lint incorrect-shift
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract IncorrectShiftBitMask {
    function singleBit(uint256 index, uint256 mask) external pure returns (uint256 result) {
        assembly ("memory-safe") {
            result := shl(index, 1)
            result := or(result, shl(add(index, 1), 0x01))
            result := and(result, and(shl(index, 1), mask))
        }
    }

    function suspiciousShifts(uint256 index) external pure returns (uint256 result) {
        assembly ("memory-safe") {
            result := shl(index, 8) //~WARN: the order of args in a shift operation is incorrect
            result := shr(index, 1) //~WARN: the order of args in a shift operation is incorrect
            result := sar(index, 1) //~WARN: the order of args in a shift operation is incorrect
            result := shl(shr(index, 8), 1) //~WARN: the order of args in a shift operation is incorrect
        }
    }
}
