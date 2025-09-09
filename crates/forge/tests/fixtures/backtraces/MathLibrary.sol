// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @title MathLibrary - A library for safe math operations
library MathLibrary {
    error DivisionByZero();
    error Underflow();
    error InvalidPercentage();
    
    /// @notice Safe division that reverts on division by zero
    function safeDiv(uint256 a, uint256 b) internal pure returns (uint256) {
        if (b == 0) revert DivisionByZero();
        return a / b;
    }
    
    /// @notice Safe subtraction that reverts on underflow
    function safeSub(uint256 a, uint256 b) internal pure returns (uint256) {
        if (b > a) revert Underflow();
        return a - b;
    }
    
    /// @notice Calculate percentage (0-100) of an amount
    function calculatePercentage(uint256 amount, uint256 percentage) internal pure returns (uint256) {
        if (percentage > 100) revert InvalidPercentage();
        return (amount * percentage) / 100;
    }
    
    /// @notice Function that uses require
    function requirePositive(uint256 value) internal pure returns (uint256) {
        require(value > 0, "Value must be positive");
        return value;
    }
}