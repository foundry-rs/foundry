// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "../src/test.sol";
import "../src/LibraryConsumer.sol";

contract LibraryBacktraceTest is DSTest {
    LibraryConsumer consumer;

    function setUp() public {
        consumer = new LibraryConsumer();
    }

    /// @notice Test division by zero in MathLibrary
    function testLibraryDivisionByZero() public {
        consumer.divide(100, 0);
    }

    /// @notice Test underflow in MathLibrary
    function testLibraryUnderflow() public {
        consumer.subtract(10, 20);
    }

    /// @notice Test invalid percentage in MathLibrary
    function testLibraryInvalidPercentage() public {
        consumer.getPercentage(1000, 150);
    }

    /// @notice Test empty string in StringLibrary (multiple libraries in one file)
    function testEmptyStringReverts() public {
        consumer.processText("");
    }

    /// @notice Test zero number in NumberLibrary (multiple libraries in one file)
    function testZeroNumberReverts() public {
        consumer.processNumber(0);
    }

    /// @notice Test complex calculation that fails in library
    function testComplexCalculationFailure() public {
        // This will fail at the division step because step1 will be 0
        consumer.complexCalculation(50, 50, 0);
    }
}
