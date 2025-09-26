// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @title ExternalMathLib - A library with external functions that needs separate deployment
library ExternalMathLib {
    error DivisionByZero();
    error Overflow();
    error Underflow();

    /// @notice External division function
    function div(uint256 a, uint256 b) external pure returns (uint256) {
        if (b == 0) revert DivisionByZero();
        return a / b;
    }

    /// @notice External multiplication with overflow check
    function mul(uint256 a, uint256 b) external pure returns (uint256) {
        if (a == 0) return 0;
        uint256 c = a * b;
        if (c / a != b) revert Overflow();
        return c;
    }

    /// @notice External subtraction with underflow check
    function sub(uint256 a, uint256 b) external pure returns (uint256) {
        if (b > a) revert Underflow();
        return a - b;
    }

    /// @notice External function with require statement
    function requirePositive(uint256 value) external pure returns (uint256) {
        require(value > 0, "ExternalMathLib: value must be positive");
        return value * 2;
    }
}
