// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./libraries/MultipleLibraries.sol";

/// @title MultipleLibraryConsumer - A contract that uses multiple libraries from the same file
contract MultipleLibraryConsumer {
    using FirstMathLib for uint256;
    using SecondMathLib for uint256;
    using ThirdMathLib for uint256;

    uint256 public result;

    /// @notice Test division from FirstMathLib
    function useFirstLib(uint256 a, uint256 b) public returns (uint256) {
        result = a.divide(b); // Should show FirstMathLib in backtrace
        return result;
    }

    /// @notice Test subtraction from SecondMathLib
    function useSecondLib(uint256 a, uint256 b) public returns (uint256) {
        result = a.subtract(b); // Should show SecondMathLib in backtrace
        return result;
    }

    /// @notice Test modulo from ThirdMathLib
    function useThirdLib(uint256 a, uint256 b) public returns (uint256) {
        result = a.modulo(b); // Should show ThirdMathLib in backtrace
        return result;
    }

    /// @notice Complex calculation using all three libraries
    function useAllLibraries(uint256 a, uint256 b, uint256 c) public returns (uint256) {
        uint256 step1 = a.divide(b); // FirstMathLib
        uint256 step2 = step1.add(c); // SecondMathLib
        result = step2.modulo(10); // ThirdMathLib
        return result;
    }
}
