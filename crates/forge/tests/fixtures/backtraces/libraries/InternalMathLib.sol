// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @title InternalMathLib - A library with internal functions that gets inlined
library InternalMathLib {
    error DivisionByZero();
    error Overflow();
    error Underflow();

    /// @notice Internal division function
    function div(uint256 a, uint256 b) internal pure returns (uint256) {
        if (b == 0) revert DivisionByZero();
        return a / b;
    }

    /// @notice Internal multiplication with overflow check
    function mul(uint256 a, uint256 b) internal pure returns (uint256) {
        if (a == 0) return 0;
        uint256 c = a * b;
        if (c / a != b) revert Overflow();
        return c;
    }

    /// @notice Internal subtraction with underflow check
    function sub(uint256 a, uint256 b) internal pure returns (uint256) {
        if (b > a) revert Underflow();
        return a - b;
    }

    /// @notice Internal function with require statement
    function requirePositive(uint256 value) internal pure returns (uint256) {
        require(value > 0, "InternalMathLib: value must be positive");
        return value * 2;
    }
}
