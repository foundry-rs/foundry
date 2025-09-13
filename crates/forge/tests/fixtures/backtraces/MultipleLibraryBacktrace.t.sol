// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "../src/test.sol";
import "../src/MultipleLibraryConsumer.sol";

contract MultipleLibraryBacktraceTest is DSTest {
    MultipleLibraryConsumer consumer;

    function setUp() public {
        consumer = new MultipleLibraryConsumer();
    }

    /// @notice Test that FirstMathLib shows correctly in backtrace
    function testFirstLibraryError() public {
        consumer.useFirstLib(10, 0); // Division by zero in FirstMathLib
    }

    /// @notice Test that SecondMathLib shows correctly in backtrace
    function testSecondLibraryError() public {
        consumer.useSecondLib(5, 10); // Underflow in SecondMathLib
    }

    /// @notice Test that ThirdMathLib shows correctly in backtrace
    function testThirdLibraryError() public {
        consumer.useThirdLib(10, 0); // Modulo by zero in ThirdMathLib
    }

    /// @notice Test complex failure in the first library
    function testAllLibrariesFirstFails() public {
        consumer.useAllLibraries(10, 0, 5); // Division by zero in FirstMathLib
    }
}
