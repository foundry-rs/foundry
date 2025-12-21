// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "../src/test.sol";
import "../src/Vm.sol";
import "../src/LibraryConsumer.sol";
import "../src/libraries/ExternalMathLib.sol";

contract LibraryBacktraceTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    LibraryConsumer consumer;
    address constant EXTERNAL_LIB_ADDRESS = 0x1234567890123456789012345678901234567890;

    function setUp() public {
        // Deploy the external library at the configured address
        bytes memory libraryBytecode = type(ExternalMathLib).runtimeCode;
        vm.etch(EXTERNAL_LIB_ADDRESS, libraryBytecode);

        // Deploy consumer contract
        consumer = new LibraryConsumer();
    }

    // Internal library tests (should show inlined source locations)

    /// @notice Test division by zero in internal library
    function testInternalDivisionByZero() public {
        consumer.internalDivide(100, 0);
    }

    /// @notice Test underflow in internal library
    function testInternalUnderflow() public {
        consumer.internalSubtract(10, 20);
    }

    /// @notice Test overflow in internal library
    function testInternalOverflow() public {
        consumer.internalMultiply(type(uint256).max, 2);
    }

    /// @notice Test require in internal library
    function testInternalRequire() public {
        consumer.internalCheckPositive(0);
    }

    // External library tests (should show delegatecall to library address)

    /// @notice Test division by zero in external library
    function testExternalDivisionByZero() public {
        consumer.externalDivide(100, 0);
    }

    /// @notice Test underflow in external library
    function testExternalUnderflow() public {
        consumer.externalSubtract(10, 20);
    }

    /// @notice Test overflow in external library
    function testExternalOverflow() public {
        consumer.externalMultiply(type(uint256).max, 2);
    }

    /// @notice Test require in external library
    function testExternalRequire() public {
        consumer.externalCheckPositive(0);
    }

    // Mixed library usage test

    /// @notice Test mixed library usage with failure in external library
    function testMixedLibraryFailure() public {
        // This will fail at the external library division step (50 - 50 = 0, then divide by 0)
        consumer.mixedCalculation(50, 50, 0);
    }
}
