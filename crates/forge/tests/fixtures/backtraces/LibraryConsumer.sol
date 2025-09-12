// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./libraries/InternalMathLib.sol";
import "./libraries/ExternalMathLib.sol";

/// @title LibraryConsumer - A contract that uses both internal and external libraries
contract LibraryConsumer {
    using InternalMathLib for uint256;

    uint256 public result;

    // Internal library functions (inlined into contract bytecode)

    /// @notice Perform division using internal library
    function internalDivide(uint256 a, uint256 b) public returns (uint256) {
        result = a.div(b);
        return result;
    }

    /// @notice Perform multiplication using internal library
    function internalMultiply(uint256 a, uint256 b) public returns (uint256) {
        result = a.mul(b);
        return result;
    }

    /// @notice Perform subtraction using internal library
    function internalSubtract(uint256 a, uint256 b) public returns (uint256) {
        result = a.sub(b);
        return result;
    }

    /// @notice Check positive value using internal library
    function internalCheckPositive(uint256 value) public returns (uint256) {
        result = InternalMathLib.requirePositive(value);
        return result;
    }

    // External library functions (delegatecall to deployed library)

    /// @notice Perform division using external library
    function externalDivide(uint256 a, uint256 b) public returns (uint256) {
        result = ExternalMathLib.div(a, b);
        return result;
    }

    /// @notice Perform multiplication using external library
    function externalMultiply(uint256 a, uint256 b) public returns (uint256) {
        result = ExternalMathLib.mul(a, b);
        return result;
    }

    /// @notice Perform subtraction using external library
    function externalSubtract(uint256 a, uint256 b) public returns (uint256) {
        result = ExternalMathLib.sub(a, b);
        return result;
    }

    /// @notice Check positive value using external library
    function externalCheckPositive(uint256 value) public returns (uint256) {
        result = ExternalMathLib.requirePositive(value);
        return result;
    }

    // Mixed usage example

    /// @notice Complex calculation using both libraries
    function mixedCalculation(uint256 a, uint256 b, uint256 c) public returns (uint256) {
        // First use internal library
        uint256 step1 = a.sub(b);
        // Then use external library
        uint256 step2 = ExternalMathLib.div(step1, c);
        // Back to internal
        result = step2.mul(10);
        return result;
    }
}
