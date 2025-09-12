// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @title FirstMathLib - First library in the file
library FirstMathLib {
    error FirstLibError();

    function divide(uint256 a, uint256 b) internal pure returns (uint256) {
        if (b == 0) {
            revert FirstLibError();
        }
        return a / b;
    }

    function multiply(uint256 a, uint256 b) internal pure returns (uint256) {
        return a * b;
    }
}

/// @title SecondMathLib - Second library in the same file
library SecondMathLib {
    error SecondLibError();

    function subtract(uint256 a, uint256 b) internal pure returns (uint256) {
        if (b > a) {
            revert SecondLibError();
        }
        return a - b;
    }

    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}

/// @title ThirdMathLib - Third library in the same file
library ThirdMathLib {
    error ThirdLibError();

    function modulo(uint256 a, uint256 b) internal pure returns (uint256) {
        if (b == 0) {
            revert ThirdLibError();
        }
        return a % b;
    }

    function power(uint256 base, uint256 exp) internal pure returns (uint256) {
        uint256 result = 1;
        for (uint256 i = 0; i < exp; i++) {
            result *= base;
        }
        return result;
    }
}
