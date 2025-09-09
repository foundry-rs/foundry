// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./MathLibrary.sol";
import "./UtilityLibraries.sol";

/// @title LibraryConsumer - A contract that uses multiple libraries
contract LibraryConsumer {
    using MathLibrary for uint256;
    using StringLibrary for string;
    using NumberLibrary for uint256;

    uint256 public result;

    /// @notice Divide two numbers using the library
    function divide(uint256 a, uint256 b) public returns (uint256) {
        result = MathLibrary.safeDiv(a, b);
        return result;
    }

    /// @notice Subtract two numbers using the library
    function subtract(uint256 a, uint256 b) public returns (uint256) {
        result = MathLibrary.safeSub(a, b);
        return result;
    }

    /// @notice Get percentage using the library
    function getPercentage(
        uint256 amount,
        uint256 percentage
    ) public returns (uint256) {
        result = MathLibrary.calculatePercentage(amount, percentage);
        return result;
    }

    /// @notice Process text using StringLibrary
    function processText(
        string memory text
    ) public pure returns (string memory) {
        return StringLibrary.requireNonEmpty(text);
    }

    /// @notice Process number using NumberLibrary
    function processNumber(uint256 num) public pure returns (uint256) {
        return NumberLibrary.requireNonZero(num);
    }

    /// @notice Complex calculation that may fail at different points
    function complexCalculation(
        uint256 a,
        uint256 b,
        uint256 c
    ) public returns (uint256) {
        uint256 step1 = MathLibrary.safeSub(a, b);
        uint256 step2 = MathLibrary.safeDiv(step1, c);
        uint256 step3 = MathLibrary.calculatePercentage(step2, 50);
        result = step3;
        return result;
    }
}
